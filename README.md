# argoshot

The official Rust client for [Argoshot](https://argoshot.com), the screenshot API: screenshots, PDFs and signed
`og:image` URLs of any web page or HTML. **Free to start.** Docs: https://argoshot.com/docs

Blocking, small: `ureq` for HTTP (rustls), `serde_json`, `hmac` + `sha2`.

## Install

```sh
cargo add argoshot
```

Get your access key (and secret key, for signed URLs) from the dashboard's API keys page.

## Screenshot to a file

```rust
use argoshot::{Client, Options};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(&std::env::var("ARGOSHOT_ACCESS_KEY")?);

    let png = client.take(
        &Options::new()
            .url("https://example.com")
            .format("png")
            .full_page(true)
            .block_cookie_banners(true),
    )?;
    std::fs::write("example.png", png)?;
    Ok(())
}
```

## Signed og:image URL

`url()` builds a `GET /take` link for `<img src>` or `<meta property="og:image">`. With a secret key it is signed
(HMAC-SHA256 of the query string), so nobody can change its options. Set the key to **signed requests only** in the
dashboard before you put its links in public pages.

```rust
let client = Client::new(&std::env::var("ARGOSHOT_ACCESS_KEY")?)
    .with_secret_key(&std::env::var("ARGOSHOT_SECRET_KEY")?);

let og_image = client.url(
    &Options::new()
        .url("https://example.com/blog/my-post")
        .viewport_width(1200)
        .viewport_height(630)
        .format("png")
        .cache(true)
        .cache_ttl(86_400),
);
// <meta property="og:image" content="{og_image}">
```

## HTML to PNG

When `html` (or `markdown`) is given, `take()` sends a JSON `POST` instead of a long query string.

```rust
let card = client.take(
    &Options::new()
        .html(r#"<div style="font: 64px system-ui; padding: 80px">Hello from Argoshot</div>"#)
        .format("png")
        .viewport_width(1200)
        .viewport_height(630),
)?;
std::fs::write("card.png", card)?;
```

## Options

Typed setters for the common options (`url`, `html`, `markdown`, `format`, `viewport_width`, `viewport_height`,
`viewport_device`, `full_page`, `block_cookie_banners`, `block_ads`, `dark_mode`, `selector`, `cache`, `cache_ttl`); any
other option of https://argoshot.com/docs/options by its name with `set`:
`.set("hide_selectors", vec![".banner", "#chat"]).set("delay", 2)`.

## Errors

`take()` returns `Err(argoshot::Error::Api(Box<ApiError>))` when the API answers an error: its `code` (e.g.
`selector_not_found`, `screenshots_limit_reached`), `message`, `status`, `documentation_url`, and `upgrade_url` /
`retry_after` when the API gives them; `Error::Transport` when there was no answer. All codes:
https://argoshot.com/docs/errors

```rust
match client.take(&Options::new().url("https://example.com").selector("#missing").set("error_on_selector_not_found", true)) {
    Ok(bytes) => std::fs::write("hero.png", bytes)?,
    Err(argoshot::Error::Api(e)) => eprintln!("{}: {}", e.code, e.message),
    Err(e) => return Err(e.into()),
}
```

## Usage

```rust
let usage = client.usage()?; // serde_json::Value: total, available, used, ...
```

## Configuration

```rust
Client::new(access_key)
    .with_secret_key("...")                      // sign url() and GET take() requests
    .with_base_url("https://api.argoshot.com")   // default
    .with_timeout(std::time::Duration::from_secs(120)); // default
```

Note: a key set to *signed requests only* cannot send JSON `POST` requests (the API refuses them), so `take()` with
`html` needs a key that permits unsigned requests.

## Licence

MIT
