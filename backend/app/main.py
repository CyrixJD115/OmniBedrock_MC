from __future__ import annotations

from contextlib import asynccontextmanager

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import HTMLResponse

from backend.app.core.auth import init_users
from backend.app.core.config import settings
from backend.app.core.dependencies import backup_scheduler, server_manager
from backend.app.core.logging import setup_logging
from backend.app.managers.performance_collector import PerformanceCollector
from backend.app.routers import (
    addons,
    audit,
    auth,
    backups,
    console,
    files,
    performance,
    players,
    properties,
    server,
    worlds,
)
from backend.app.routers import (
    settings as settings_router,
)


@asynccontextmanager
async def lifespan(app: FastAPI):
    setup_logging()
    init_users()
    collector = PerformanceCollector()
    collector.set_server_manager(server_manager)
    await collector.start()
    await backup_scheduler.start()

    print(f"\n{'='*50}")
    print("  OmniBedrock MC Panel")
    import os
    port = os.getenv("OMNI_PORT", "17754")
    print(f"  API: http://localhost:{port}{settings.api_prefix}")
    print(f"{'='*50}\n")

    yield

    await backup_scheduler.stop()
    await collector.stop()
    await server_manager.kill()


def create_app() -> FastAPI:
    app = FastAPI(
        title=settings.app_name,
        version="0.1.0",
        lifespan=lifespan,
    )

    app.add_middleware(
        CORSMiddleware,
        allow_origins=settings.cors_origins,
        allow_credentials=True,
        allow_methods=["*"],
        allow_headers=["*"],
    )

    prefix = settings.api_prefix
    app.include_router(auth.router, prefix=prefix)
    app.include_router(audit.router, prefix=prefix)
    app.include_router(server.router, prefix=prefix)
    app.include_router(console.router, prefix=prefix)
    app.include_router(properties.router, prefix=prefix)
    app.include_router(backups.router, prefix=prefix)
    app.include_router(addons.router, prefix=prefix)
    app.include_router(worlds.router, prefix=prefix)
    app.include_router(players.router, prefix=prefix)
    app.include_router(settings_router.router, prefix=prefix)
    app.include_router(files.router, prefix=prefix)
    app.include_router(performance.router, prefix=prefix)

    @app.get("/", response_class=HTMLResponse, include_in_schema=False)
    async def root():
        return """<!DOCTYPE html>
<html lang="en" class="dark">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>OmniBedrock MC</title>
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link href="https://fonts.googleapis.com/css2?family=Montserrat:wght@600;700;800&family=Inter:wght@400;500;600&display=swap" rel="stylesheet">
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body {
      background: #020617;
      color: #e2e8f0;
      font-family: 'Inter', system-ui, -apple-system, sans-serif;
      display: flex;
      align-items: center;
      justify-content: center;
      min-height: 100vh;
      background-image:
        radial-gradient(ellipse at 50% 0%, rgba(6, 145, 178, 0.04) 0%, transparent 60%),
        radial-gradient(ellipse at 50% 100%, rgba(45, 212, 191, 0.03) 0%, transparent 60%);
    }
    .card {
      background: #0f172a;
      border: 2px solid #1a3050;
      padding: 48px;
      max-width: 420px;
      text-align: center;
      box-shadow:
        inset 2px 2px 0 rgba(255,255,255,0.03),
        inset -1px -1px 0 rgba(0,0,0,0.3),
        6px 6px 0 rgba(0,0,0,0.5);
    }
    .pixel-divider {
      height: 2px;
      margin: 28px 0;
      background: repeating-linear-gradient(
        90deg,
        #1a3050 0px,
        #1a3050 4px,
        transparent 4px,
        transparent 8px
      );
    }
    .icon-grid {
      width: 56px;
      height: 56px;
      margin: 0 auto 20px;
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 2px;
    }
    .icon-grid div {
      border-radius: 2px;
    }
    .icon-grid .tl { background: #2dd4bf; box-shadow: 0 0 12px rgba(45,212,191,0.4); }
    .icon-grid .tr { background: #0e7490; }
    .icon-grid .bl { background: #0891b2; }
    .icon-grid .br { background: #14b8a6; }
    h1 {
      font-family: 'Montserrat', system-ui, sans-serif;
      font-size: 26px;
      font-weight: 800;
      color: #fff;
      letter-spacing: 0.05em;
      text-transform: uppercase;
      text-shadow: 0 0 20px rgba(34, 211, 238, 0.15);
    }
    .subtitle {
      font-size: 12px;
      color: #5a7184;
      text-transform: uppercase;
      letter-spacing: 0.15em;
      margin-top: 6px;
      font-weight: 600;
    }
    p {
      color: #5a7184;
      font-size: 13px;
      line-height: 1.7;
      margin-bottom: 0;
    }
    .url {
      color: #2dd4bf;
      font-family: 'Inter', monospace;
      font-size: 13px;
      font-weight: 600;
      display: inline-block;
      margin-top: 16px;
      padding: 8px 16px;
      border: 1px solid #2dd4bf20;
      background: rgba(45,212,191,0.06);
    }
    .badge {
      display: inline-block;
      padding: 5px 14px;
      font-family: 'Montserrat', system-ui, sans-serif;
      font-size: 11px;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.1em;
      background: rgba(6, 182, 212, 0.1);
      color: #67e8f9;
      border: 1px solid rgba(6, 182, 212, 0.2);
    }
    .status-dot {
      width: 7px;
      height: 7px;
      display: inline-block;
      margin-right: 6px;
      background: #2dd4bf;
      box-shadow: 0 0 8px rgba(45,212,191,0.6);
      animation: pulse 2s infinite;
    }
    @keyframes pulse {
      0%, 100% { opacity: 1; }
      50% { opacity: 0.4; }
    }
    .footer {
      margin-top: 32px;
      font-size: 10px;
      color: #334155;
      text-transform: uppercase;
      letter-spacing: 0.2em;
    }
  </style>
</head>
<body>
  <div class="card">
    <div class="icon-grid">
      <div class="tl"></div>
      <div class="tr"></div>
      <div class="bl"></div>
      <div class="br"></div>
    </div>
    <h1>OmniBedrock MC</h1>
    <div class="subtitle">Server Control Panel</div>
    <div class="pixel-divider"></div>
    <p>Backend API is running.</p>
    <span class="url">http://localhost:17755</span>
    <div class="pixel-divider"></div>
    <span class="badge"><span class="status-dot"></span>API v0.1.0</span>
    <div class="footer">Point your browser to the frontend</div>
  </div>
</body>
</html>"""

    return app


app = create_app()
