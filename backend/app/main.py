from __future__ import annotations

from contextlib import asynccontextmanager

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware

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
    print(f"  API: http://localhost:8000{settings.api_prefix}")
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

    return app


app = create_app()
