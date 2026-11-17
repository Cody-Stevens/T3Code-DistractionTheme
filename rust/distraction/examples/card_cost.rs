//! Isolated native card timing; no audio device or T3 access.
use dt_render::card::{CardContent, render};
use std::time::Instant;
fn main() {
    for second in 0..5 {
        let began = Instant::now();
        let card = render(
            &CardContent {
                title: "Polyphia - POWER IN THE BLOOD (Official Music Video)".into(),
                subtitle: format!("Polyphia  -  0:{second:02} / 4:01"),
                playing: true,
                upcoming: vec![],
            },
            &distraction::runtime::card_options(),
        )
        .unwrap();
        println!(
            "card {second}: {:.3}s, {} base64 characters",
            began.elapsed().as_secs_f64(),
            card.image.len()
        );
    }
}
