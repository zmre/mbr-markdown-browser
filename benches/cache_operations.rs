//! Benchmarks for OEmbed cache operations.
//!
//! Measures get (hit/miss) and insert performance including eviction pressure.

use criterion::{Criterion, criterion_group, criterion_main};
use mbr::oembed::PageInfo;
use mbr::oembed_cache::OembedCache;

fn make_page_info(url: &str) -> PageInfo {
    PageInfo {
        url: url.to_string(),
        title: Some("Benchmark Page Title".to_string()),
        description: Some("A description for benchmarking cache performance.".to_string()),
        image: Some("https://example.com/image.jpg".to_string()),
        embed_html: None,
    }
}

fn bench_cache_get_hit(c: &mut Criterion) {
    let cache = OembedCache::new(1024 * 1024); // 1MB

    // Pre-populate with 100 entries
    for i in 0..100 {
        let url = format!("https://example.com/page/{}", i);
        cache.insert(
            url,
            make_page_info(&format!("https://example.com/page/{}", i)),
        );
    }

    c.bench_function("cache_get_hit", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let url = format!("https://example.com/page/{}", i % 100);
            i += 1;
            cache.get(&url)
        });
    });
}

fn bench_cache_get_miss(c: &mut Criterion) {
    let cache = OembedCache::new(1024 * 1024);

    // Pre-populate with entries that won't match our queries
    for i in 0..100 {
        let url = format!("https://example.com/cached/{}", i);
        cache.insert(
            url,
            make_page_info(&format!("https://example.com/cached/{}", i)),
        );
    }

    c.bench_function("cache_get_miss", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let url = format!("https://example.com/uncached/{}", i);
            i += 1;
            cache.get(&url)
        });
    });
}

fn bench_cache_insert(c: &mut Criterion) {
    c.bench_function("cache_insert", |b| {
        let cache = OembedCache::new(1024 * 1024);
        let mut i = 0u64;
        b.iter(|| {
            let url = format!("https://example.com/insert/{}", i);
            let info = make_page_info(&url);
            i += 1;
            cache.insert(url, info);
        });
    });
}

fn bench_cache_insert_with_eviction(c: &mut Criterion) {
    // Small cache that forces frequent eviction
    c.bench_function("cache_insert_eviction", |b| {
        let cache = OembedCache::new(2048); // Very small — forces eviction
        let mut i = 0u64;
        b.iter(|| {
            let url = format!("https://example.com/evict/{}", i);
            let info = make_page_info(&url);
            i += 1;
            cache.insert(url, info);
        });
    });
}

criterion_group!(
    benches,
    bench_cache_get_hit,
    bench_cache_get_miss,
    bench_cache_insert,
    bench_cache_insert_with_eviction
);
criterion_main!(benches);
