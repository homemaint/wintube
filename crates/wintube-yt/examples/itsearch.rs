//! Quick probe for the InnerTube search path.
//!
//! Usage: cargo run --release --example itsearch -p wintube-yt -- "query"

#[tokio::main]
async fn main() {
    let q = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "trending music".to_string());
    let t = std::time::Instant::now();
    match wintube_yt::innertube::search(&q).await {
        Ok(cards) => {
            println!("query={q:?} -> {} cards in {}ms", cards.len(), t.elapsed().as_millis());
            for c in cards.iter().take(4) {
                println!(
                    "  {} | {} | {} | dur={:?} | views={:?} | thumb={}",
                    c.id,
                    c.title,
                    c.channel,
                    c.duration,
                    c.views,
                    &c.thumbnail[..c.thumbnail.len().min(70)]
                );
            }
        }
        Err(e) => println!("FAILED after {}ms: {e}", t.elapsed().as_millis()),
    }
}
