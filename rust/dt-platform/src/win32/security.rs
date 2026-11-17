use super::{error, owned, raw};
use dt_core::{Error, Result};
use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt, path::Path};
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        Storage::FileSystem::*,
        System::Threading::*,
    },
    core::PWSTR,
};

struct LocalAllocation(*mut core::ffi::c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: pointer was allocated by SetEntriesInAclW/GetSecurityInfo and is freed exactly once.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0)));
        }
    }
}

fn user_token() -> Result<Vec<usize>> {
    // SAFETY: open transfers ownership. GetTokenInformation first determines size; Vec<usize>
    // supplies sufficient alignment for TOKEN_USER/SID. The returned SID points inside that Vec.
    unsafe {
        let mut handle = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle).map_err(error)?;
        let token = owned(handle);
        let mut bytes = 0;
        let _ = GetTokenInformation(raw(&token), TokenUser, None, 0, &mut bytes);
        if bytes == 0 {
            return Err(Error::Protocol("token user size unavailable"));
        }
        let mut buffer = vec![0usize; (bytes as usize).div_ceil(std::mem::size_of::<usize>())];
        GetTokenInformation(
            raw(&token),
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            bytes,
            &mut bytes,
        )
        .map_err(error)?;
        Ok(buffer)
    }
}

pub(crate) fn restrict_to_owner(path: &Path) -> Result<()> {
    // Open the object itself, never a symlink/junction target. Deny rename while applying ACL.
    let file = OpenOptions::new()
        .access_mode((READ_CONTROL | WRITE_DAC | WRITE_OWNER).0)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
        .custom_flags((FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).0)
        .open(path)?;
    let user = user_token()?;
    // SAFETY: TOKEN_USER and SID remain backed by aligned `user`; all ACL allocations are
    // guarded. SetSecurityInfo copies the DACL/owner to the already-open object before returning.
    unsafe {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        GetFileInformationByHandle(raw(&file), &mut info).map_err(error)?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err(Error::InvalidInput("token path is a reparse point"));
        }
        let sid = (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_ALL_ACCESS.0,
            grfAccessMode: SET_ACCESS,
            grfInheritance: if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
                SUB_CONTAINERS_AND_OBJECTS_INHERIT
            } else {
                ACE_FLAGS(0)
            },
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_USER,
                ptstrName: PWSTR(sid.0.cast()),
                ..Default::default()
            },
        };
        let mut acl = std::ptr::null_mut();
        SetEntriesInAclW(Some(&[entry]), None, &mut acl)
            .ok()
            .map_err(error)?;
        let _acl = LocalAllocation(acl.cast());
        SetSecurityInfo(
            raw(&file),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            Some(sid),
            None,
            Some(acl),
            None,
        )
        .ok()
        .map_err(error)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn verify_owner_acl(path: &Path, protected: bool) -> Result<bool> {
    let file = OpenOptions::new()
        .access_mode(READ_CONTROL.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)?;
    let user = user_token()?;
    // SAFETY: GetSecurityInfo owns a descriptor until LocalFree; owner, DACL and ACE borrow it.
    // ACE offsets are read only after confirming a single ACCESS_ALLOWED ACE of sufficient size.
    unsafe {
        let mut owner = PSID::default();
        let mut acl = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        GetSecurityInfo(
            raw(&file),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut acl),
            None,
            Some(&mut descriptor),
        )
        .ok()
        .map_err(error)?;
        let _descriptor = LocalAllocation(descriptor.0);
        let sid = (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        if acl.is_null() || (*acl).AceCount != 1 || EqualSid(owner, sid).is_err() {
            return Ok(false);
        }
        let mut control = 0u16;
        let mut revision = 0;
        GetSecurityDescriptorControl(descriptor, &mut control, &mut revision).map_err(error)?;
        if protected && control & SE_DACL_PROTECTED.0 == 0 {
            return Ok(false);
        }
        let mut ace = std::ptr::null_mut();
        GetAce(acl, 0, &mut ace).map_err(error)?;
        let header = &*ace.cast::<ACE_HEADER>();
        if header.AceType != 0 /* ACCESS_ALLOWED_ACE_TYPE */ || (header.AceSize as usize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
        {
            return Ok(false);
        }
        let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
        Ok(allowed.Mask == FILE_ALL_ACCESS.0
            && EqualSid(
                PSID((&allowed.SidStart as *const u32).cast_mut().cast()),
                sid,
            )
            .is_ok())
    }
}
