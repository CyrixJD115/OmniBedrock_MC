//! `GET /api-docs` — a zero-dependency human page: links the OpenAPI
//! document and the route list, and renders the routes client-side from
//! `{prefix}` so the page can never drift from the served API. Public, like
//! the machine-readable endpoints it links.
use axum::{
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};

use super::state::AppState;

const TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>OmniBedrock Panel API</title>
<style>
  :root { color-scheme: light dark; }
  body { font: 14px/1.5 system-ui, sans-serif; max-width: 60rem; margin: 2rem auto; padding: 0 1rem; }
  h1 { font-size: 1.3rem; } h2 { font-size: 1.05rem; margin-top: 2rem; }
  code { background: color-mix(in srgb, currentColor 8%, transparent); padding: .1rem .35rem; border-radius: .3rem; }
  table { border-collapse: collapse; width: 100%; }
  td, th { text-align: left; padding: .3rem .6rem; border-bottom: 1px solid color-mix(in srgb, currentColor 15%, transparent); vertical-align: top; }
  td.method { font-family: ui-monospace, monospace; font-weight: 600; white-space: nowrap; }
  td.path { font-family: ui-monospace, monospace; white-space: nowrap; }
  .muted { opacity: .65; }
</style>
</head>
<body>
<h1>OmniBedrock Panel API</h1>
<p>
  REST API for instances, console, backups, addons, store, and broadcaster.
  Authentication: <code>Authorization: Bearer &lt;token&gt;</code> from
  <code>POST __PREFIX__/auth/login</code> (instance-scoped calls add
  <code>X-Instance: &lt;id&gt;</code>). Every response carries an
  <code>X-Request-Id</code> header; JSON objects also carry
  <code>"ok"</code>, and errors add <code>"error": {"code", "message"}</code>.
</p>
<h2>Machine-readable</h2>
<ul>
  <li><a href="/openapi.json">/openapi.json</a> — OpenAPI 3.0 document <span class="muted">(also at <code>__PREFIX__/openapi.json</code>)</span></li>
  <li><a href="__PREFIX__">__PREFIX__</a> — route list as a JSON array of <code>{method, path, summary}</code> <span class="muted">(append <code>?format=text</code> for plain text)</span></li>
  <li><a href="/health">/health</a> — liveness</li>
</ul>
<h2>Routes</h2>
<table id="routes">
  <thead><tr><th>Method</th><th>Path</th><th>Summary</th></tr></thead>
  <tbody><tr><td colspan="3" class="muted">Loading __PREFIX__ …</td></tr></tbody>
</table>
<script>
fetch('__PREFIX__', {headers: {'Accept': 'application/json'}})
  .then(r => r.json())
  .then(routes => {
    const body = document.querySelector('#routes tbody');
    body.innerHTML = routes.map(r =>
      '<tr><td class="method">' + r.method + '</td><td class="path">' +
      r.path + '</td><td>' + r.summary + '</td></tr>').join('');
  })
  .catch(() => {
    document.querySelector('#routes tbody').innerHTML =
      '<tr><td colspan="3">Could not load the route list.</td></tr>';
  });
</script>
</body>
</html>
"#;

pub(super) async fn api_docs(State(state): State<AppState>) -> Response {
    let html = TEMPLATE.replace("__PREFIX__", &state.config.api_prefix);
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
}
