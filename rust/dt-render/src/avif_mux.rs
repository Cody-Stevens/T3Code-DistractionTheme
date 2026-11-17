//! Minimal AVIF image-sequence muxer: primary still item plus a timed AV1 image track.
//! Packets are low-overhead AV1 OBUs, beginning with an independently decodable key frame.
use dt_core::{Error, Result};
fn u16s(v: u16) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}
fn u32s(v: u32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}
fn u64s(v: u64) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}
fn bx(kind: &[u8; 4], data: Vec<u8>) -> Vec<u8> {
    [u32s((data.len() + 8) as u32), kind.to_vec(), data].concat()
}
fn full(kind: &[u8; 4], flags: u32, data: Vec<u8>) -> Vec<u8> {
    bx(kind, [u32s(flags), data].concat())
}
fn matrix() -> Vec<u8> {
    [0x10000_u32, 0, 0, 0, 0x10000, 0, 0, 0, 0x40000000]
        .into_iter()
        .flat_map(u32s)
        .collect()
}
fn handler() -> Vec<u8> {
    full(
        b"hdlr",
        0,
        [u32s(0), b"pict".to_vec(), vec![0; 13]].concat(),
    )
}
fn colr() -> Vec<u8> {
    bx(
        b"colr",
        [b"nclx".to_vec(), u16s(1), u16s(13), u16s(6), vec![128]].concat(),
    )
}
fn av1c(packet: &[u8]) -> Result<Vec<u8>> {
    let mut offset = 0;
    while offset < packet.len() {
        let h = packet[offset];
        offset += 1;
        if h & 4 != 0 {
            offset += 1;
        }
        if h & 2 == 0 {
            return Err(Error::Protocol("AV1 OBU must have length"));
        }
        let mut size = 0_usize;
        let mut shift = 0;
        loop {
            let b = *packet
                .get(offset)
                .ok_or(Error::Protocol("truncated AV1 OBU"))?;
            offset += 1;
            size |= usize::from(b & 127) << shift;
            if b & 128 == 0 {
                break;
            }
            shift += 7;
            if shift > 28 {
                return Err(Error::Protocol("invalid AV1 OBU length"));
            }
        }
        let end = offset
            .checked_add(size)
            .filter(|v| *v <= packet.len())
            .ok_or(Error::Protocol("truncated AV1 packet"))?;
        if (h >> 3) & 15 == 1 {
            let data = &packet[offset..end];
            let mut pos = 0;
            let mut bits = |n: usize| -> Result<u32> {
                let mut v = 0;
                for _ in 0..n {
                    let b = data
                        .get(pos / 8)
                        .ok_or(Error::Protocol("truncated AV1 sequence header"))?;
                    v = (v << 1) | u32::from((b >> (7 - pos % 8)) & 1);
                    pos += 1;
                }
                Ok(v)
            };
            let profile = bits(3)?;
            let _still = bits(1)?;
            let reduced = bits(1)?;
            let level = if reduced != 0 {
                bits(5)?
            } else {
                if bits(1)? != 0 {
                    return Err(Error::Protocol("unexpected AV1 timing model"));
                }
                let _delay = bits(1)?;
                let _ops = bits(5)?;
                let _id = bits(12)?;
                bits(5)?
            };
            if profile != 0 {
                return Err(Error::Protocol(
                    "AVIF muxer requires 8-bit 4:2:0 main profile",
                ));
            }
            return Ok(bx(b"av1C", vec![0x81, level as u8, 0x0c, 0]));
        }
        offset = end;
    }
    Err(Error::Protocol("AV1 key frame has no sequence header"))
}
/// Mux with a 1000-unit timescale and exact positive millisecond durations.
pub(super) fn mux(
    packets: &[Vec<u8>],
    width: u32,
    height: u32,
    durations: &[u32],
    plays: u32,
) -> Result<Vec<u8>> {
    if packets.is_empty()
        || packets.len() != durations.len()
        || packets.iter().any(Vec::is_empty)
        || durations.contains(&0)
        || width > u16::MAX.into()
        || height > u16::MAX.into()
    {
        return Err(Error::InvalidInput("invalid AVIF sequence"));
    }
    let config = av1c(&packets[0])?;
    let total = durations.iter().map(|v| u64::from(*v)).sum::<u64>();
    let ftyp = bx(
        b"ftyp",
        [
            b"avis".to_vec(),
            u32s(0),
            b"avifavismsf1iso8mif1miafMA1B".to_vec(),
        ]
        .concat(),
    );
    let make_meta = |offset: u32| {
        full(
            b"meta",
            0,
            [
                handler(),
                full(b"pitm", 0, u16s(1)),
                full(
                    b"iloc",
                    0,
                    [
                        vec![0x44, 0],
                        u16s(1),
                        u16s(1),
                        u16s(0),
                        u16s(1),
                        u32s(offset),
                        u32s(packets[0].len() as u32),
                    ]
                    .concat(),
                ),
                full(
                    b"iinf",
                    0,
                    [
                        u16s(1),
                        full(
                            b"infe",
                            0x02000000,
                            [u16s(1), u16s(0), b"av01Color\0".to_vec()].concat(),
                        ),
                    ]
                    .concat(),
                ),
                bx(
                    b"iprp",
                    [
                        bx(
                            b"ipco",
                            [
                                full(b"ispe", 0, [u32s(width), u32s(height)].concat()),
                                full(b"pixi", 0, vec![3, 8, 8, 8]),
                                config.clone(),
                                colr(),
                            ]
                            .concat(),
                        ),
                        full(
                            b"ipma",
                            0,
                            [u32s(1), u16s(1), vec![4, 1, 2, 0x83, 4]].concat(),
                        ),
                    ]
                    .concat(),
                ),
            ]
            .concat(),
        )
    };
    let make_moov = |offset: u32| {
        let duration = if plays == 0 {
            u64::MAX
        } else {
            total.saturating_mul(u64::from(plays))
        };
        let mvhd = full(
            b"mvhd",
            0x01000000,
            [
                vec![0; 16],
                u32s(1000),
                u64s(duration),
                u32s(0x10000),
                u16s(0x100),
                vec![0; 10],
                matrix(),
                vec![0; 24],
                u32s(2),
            ]
            .concat(),
        );
        let tkhd = full(
            b"tkhd",
            0x01000001,
            [
                vec![0; 16],
                u32s(1),
                u32s(0),
                u64s(duration),
                vec![0; 16],
                matrix(),
                u32s(width << 16),
                u32s(height << 16),
            ]
            .concat(),
        );
        // AVIF looping uses an edit list with the repeat flag; one play has no repeat.
        let edits = bx(
            b"edts",
            full(
                b"elst",
                0x01000000 | u32::from(plays != 1),
                [u32s(1), u64s(total), u64s(0), u16s(1), u16s(0)].concat(),
            ),
        );
        let mdhd = full(
            b"mdhd",
            0x01000000,
            [vec![0; 16], u32s(1000), u64s(total), u16s(0x55c4), u16s(0)].concat(),
        );
        let av01 = bx(
            b"av01",
            [
                vec![0; 6],
                u16s(1),
                vec![0; 16],
                u16s(width as u16),
                u16s(height as u16),
                u32s(0x480000),
                u32s(0x480000),
                u32s(0),
                u16s(1),
                vec![0; 32],
                u16s(0x18),
                u16s(0xffff),
                config.clone(),
                colr(),
                full(b"ccst", 0, u32s(0x7c000000)),
            ]
            .concat(),
        );
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for &d in durations {
            if let Some(last) = runs.last_mut().filter(|v| v.1 == d) {
                last.0 += 1;
            } else {
                runs.push((1, d));
            }
        }
        let stts = full(
            b"stts",
            0,
            [
                u32s(runs.len() as u32),
                runs.iter()
                    .flat_map(|(n, d)| [u32s(*n), u32s(*d)].concat())
                    .collect(),
            ]
            .concat(),
        );
        let stbl = bx(
            b"stbl",
            [
                full(b"stsd", 0, [u32s(1), av01].concat()),
                stts,
                full(
                    b"stsc",
                    0,
                    [u32s(1), u32s(1), u32s(packets.len() as u32), u32s(1)].concat(),
                ),
                full(
                    b"stsz",
                    0,
                    [
                        u32s(0),
                        u32s(packets.len() as u32),
                        packets.iter().flat_map(|p| u32s(p.len() as u32)).collect(),
                    ]
                    .concat(),
                ),
                full(b"stco", 0, [u32s(1), u32s(offset)].concat()),
                full(b"stss", 0, [u32s(1), u32s(1)].concat()),
            ]
            .concat(),
        );
        let dinf = bx(
            b"dinf",
            full(b"dref", 0, [u32s(1), full(b"url ", 1, vec![])].concat()),
        );
        let minf = bx(b"minf", [full(b"vmhd", 1, vec![0; 8]), dinf, stbl].concat());
        bx(
            b"moov",
            [
                mvhd,
                bx(
                    b"trak",
                    [tkhd, edits, bx(b"mdia", [mdhd, handler(), minf].concat())].concat(),
                ),
            ]
            .concat(),
        )
    };
    let offset = ftyp.len() + make_meta(0).len() + make_moov(0).len() + 8;
    let size = packets.iter().map(Vec::len).sum::<usize>();
    if offset
        .checked_add(size)
        .is_none_or(|n| n > u32::MAX as usize)
    {
        return Err(Error::InvalidInput("AVIF exceeds 32-bit container"));
    }
    Ok([
        ftyp,
        make_meta(offset as u32),
        make_moov(offset as u32),
        bx(b"mdat", packets.concat()),
    ]
    .concat())
}
