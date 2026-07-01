use axum::response::Html;

pub async fn landing_page() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>dave-wang-6c2c-daily-video</title>
  <style>
    :root {
      color-scheme: light;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      background: #f8fafc;
      color: #0f172a;
    }
    body {
      margin: 0;
      min-height: 100vh;
      display: grid;
      place-items: center;
      background:
        radial-gradient(circle at top left, rgba(14, 165, 233, 0.18), transparent 28rem),
        linear-gradient(135deg, #f8fafc 0%, #e0f2fe 100%);
    }
    main {
      width: min(56rem, calc(100vw - 2rem));
      padding: 3rem;
      border-radius: 1.5rem;
      background: rgba(255, 255, 255, 0.9);
      box-shadow: 0 24px 80px rgba(15, 23, 42, 0.16);
      border: 1px solid rgba(148, 163, 184, 0.3);
    }
    h1 {
      margin: 0 0 1rem;
      font-size: clamp(2rem, 6vw, 4rem);
      line-height: 1;
      letter-spacing: -0.05em;
    }
    p {
      margin: 0 0 1.25rem;
      color: #334155;
      font-size: 1.125rem;
      line-height: 1.7;
    }
    nav {
      display: flex;
      flex-wrap: wrap;
      gap: 0.75rem;
      margin-top: 2rem;
    }
    a {
      display: inline-block;
      padding: 0.8rem 1rem;
      border-radius: 999px;
      background: #0f172a;
      color: #fff;
      text-decoration: none;
      font-weight: 700;
    }
    a.secondary {
      background: #e2e8f0;
      color: #0f172a;
    }
  </style>
</head>
<body>
  <main>
    <h1>Daily animal video pipeline</h1>
    <p>
      This service generates a daily funny animal video, creates a 3D print-reveal
      segment, uploads final artifacts to private object storage, and exposes a
      public JSON feed for consumers.
    </p>
    <p>
      The API is online. Use the feed endpoints below to fetch published videos
      or check service health.
    </p>
    <nav aria-label="Primary API links">
      <a href="/videos">Published videos</a>
      <a href="/videos/latest" class="secondary">Latest video</a>
      <a href="/health" class="secondary">Health check</a>
    </nav>
  </main>
</body>
</html>"#,
    )
}
