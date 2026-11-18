// Renders example.com for real: runs only with ARGOSHOT_KEY set (ARGOSHOT_SECRET optional).
#[test]
fn live_png() {
    let Ok(key) = std::env::var("ARGOSHOT_KEY") else {
        eprintln!("set ARGOSHOT_KEY to run the live test");
        return;
    };
    let mut client = argoshot::Client::new(&key);
    if let Ok(secret) = std::env::var("ARGOSHOT_SECRET") {
        client = client.with_secret_key(&secret);
    }
    let png = client.take(&argoshot::Options::new().url("https://example.com").format("png")).expect("render");
    assert_eq!(&png[..4], b"\x89PNG");
}
