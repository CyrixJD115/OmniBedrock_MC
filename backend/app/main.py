from __future__ import annotations

from contextlib import asynccontextmanager

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import HTMLResponse

from backend.app.core.config import settings
from backend.app.core.logging import setup_logging
from backend.app.core.security import generate_token
from backend.app.routers import (
    server,
    console,
    properties,
    backups,
    addons,
    worlds,
    players,
    inieditor,
    settings as settings_router,
    performance,
)
from backend.app.managers.performance_collector import PerformanceCollector
from backend.app.core.dependencies import server_manager


@asynccontextmanager
async def lifespan(app: FastAPI):
    setup_logging()
    collector = PerformanceCollector()
    collector.set_server_manager(server_manager)
    await collector.start()

    token = generate_token()
    print(f"\n{'='*50}")
    print(f"  OmniBedrock MC Panel")
    import os
    port = os.getenv("OMNI_PORT", "17755")
    print(f"  API: http://localhost:{port}{settings.api_prefix}")
    print(f"  Auth Token: {token}")
    print(f"{'='*50}\n")

    yield

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
    app.include_router(server.router, prefix=prefix)
    app.include_router(console.router, prefix=prefix)
    app.include_router(properties.router, prefix=prefix)
    app.include_router(backups.router, prefix=prefix)
    app.include_router(addons.router, prefix=prefix)
    app.include_router(worlds.router, prefix=prefix)
    app.include_router(players.router, prefix=prefix)
    app.include_router(inieditor.router, prefix=prefix)
    app.include_router(settings_router.router, prefix=prefix)
    app.include_router(performance.router, prefix=prefix)

    @app.get("/", response_class=HTMLResponse, include_in_schema=False)
    async def root():
        return """<!DOCTYPE html>
<html lang="en" class="dark">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>OmniBedrock MC</title>
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body {
      background: #020617; color: #e2e8f0;
      font-family: system-ui, -apple-system, sans-serif;
      display: flex; align-items: center; justify-content: center;
      min-height: 100vh;
    }
    .card {
      background: #0f172a; border: 1px solid #334155;
      border-radius: 16px; padding: 48px; text-align: center;
      max-width: 480px;
    }
    .logo {
      width: 48px; height: 48px; border-radius: 50%;
      background: #22c55e; margin: 0 auto 24px;
      box-shadow: 0 0 24px #39ff14;
    }
    h1 { font-size: 28px; font-weight: 700; color: #fff; margin-bottom: 8px; }
    p { color: #94a3b8; margin-bottom: 24px; line-height: 1.6; }
    .url { color: #22c55e; font-family: monospace; font-size: 14px; }
    .badge {
      display: inline-block; padding: 4px 12px;
      border-radius: 999px; font-size: 12px; font-weight: 600;
      background: #22c55e20; color: #4ade80; border: 1px solid #22c55e40;
    }
  </style>
</head>
<body>
  <div class="card">
    <div class="logo"></div>
    <h1>OmniBedrock MC</h1>
    <p>Backend API running.<br>Point your browser to the frontend dev server at <span class="url">http://localhost:5173</span></p>
    <span class="badge">API v0.1.0</span>
  </div>
</body>
</html>"""

    return app


app = create_app()
