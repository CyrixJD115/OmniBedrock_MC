from __future__ import annotations

from pathlib import Path

from pydantic_settings import BaseSettings


class Settings(BaseSettings):
    app_name: str = "OmniBedrock MC"
    debug: bool = False
    secret_key: str = "change-me-in-production"
    api_prefix: str = "/api/v1"
    cors_origins: list[str] = ["http://localhost:5173", "http://localhost:3000"]

    bedrock_server_dir: str = str(Path(__file__).resolve().parent.parent.parent.parent / "bedrock_server")
    backups_dir: str = str(Path(__file__).resolve().parent.parent.parent / "backups")
    ini_dir: str = str(Path(__file__).resolve().parent.parent.parent / "ini")
    logs_dir: str = str(Path(__file__).resolve().parent.parent.parent / "logs")

    ws_heartbeat_interval: int = 30
    console_max_lines: int = 5000
    console_batch_ms: int = 120

    model_config = {"env_file": ".env", "env_file_encoding": "utf-8", "extra": "ignore"}


settings = Settings()
