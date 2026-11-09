//! Quick probe for the InnerTube search path.
//!
//! Usage: cargo run --release --example itsearch -p wintube-yt -- "query"

#[tokio::main]
async fn main() {
    let q = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "trending music".to_string());
    let t = std::time::Instant::now();
    match wintube_yt::innertube::search_page(&q, None).await {
        Ok(page) => {
            println!(
                "query={q:?} -> {} cards in {}ms, more={}",
                page.cards.len(),
                t.elapsed().as_millis(),
                page.continuation.is_some()
            );
            // Page 2 via the continuation token.
            if let Some(token) = &page.continuation {
                match wintube_yt::innertube::search_page(&q, Some(token)).await {
                    Ok(p2) => println!("  page 2 -> {} more cards, more={}", p2.cards.len(), p2.continuation.is_some()),
                    Err(e) => println!("  page 2 FAILED: {e}"),
                }
            }
            for c in page.cards.iter().take(4) {
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
