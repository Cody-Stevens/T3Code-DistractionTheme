//! Standalone benchmark, copied by tools/rav1e_probe.ps1 into rust/target/rav1e-probe.
#[path = "../../dt-render/src/avif_mux.rs"]
mod avif_mux;
use rav1e::prelude::*;
use std::{path::PathBuf,sync::Arc,time::Instant};
fn main()->Result<(),Box<dyn std::error::Error>> {
    let dir=PathBuf::from(std::env::args_os().nth(1).ok_or("benchmark directory")?);
    let raw=std::fs::read(dir.join("input.yuv"))?;
    for q in [80,130,180,220,240] {
        let mut times=Vec::new();let mut size=0;
        for _ in 0..3 {
            let start=Instant::now();let mut enc=EncoderConfig::with_speed_preset(10);enc.width=640;enc.height=360;enc.quantizer=q;enc.low_latency=true;enc.min_key_frame_interval=8;enc.max_key_frame_interval=8;enc.pixel_range=PixelRange::Full;enc.tune=Tune::Psnr;
            let mut context:Context<u8>=Config::new().with_encoder_config(enc).with_threads(4).new_context()?;
            for bytes in raw.chunks_exact(640*360*3/2) {let mut frame=context.new_frame();frame.planes[0].copy_from_raw_u8(&bytes[..640*360],640,1);frame.planes[1].copy_from_raw_u8(&bytes[640*360..640*360*5/4],320,1);frame.planes[2].copy_from_raw_u8(&bytes[640*360*5/4..],320,1);context.send_frame(Arc::new(frame))?;}
            context.flush();let mut packets=Vec::new();loop{match context.receive_packet(){Ok(packet)=>packets.push(packet.data),Err(EncoderStatus::Encoded)=>continue,Err(EncoderStatus::LimitReached)=>break,Err(e)=>return Err(format!("rav1e: {e:?}").into())}}
            let data=avif_mux::mux(&packets,640,360,&vec![42;packets.len()],1)?;times.push(start.elapsed().as_secs_f64()*1000.0);size=data.len();std::fs::write(dir.join(format!("rav1e-rust-noasm-q{q}.avif")),data)?;
        }
        println!("rav1e noasm q{q}: {size} bytes, ms {times:?}");
    }
    Ok(())
}
