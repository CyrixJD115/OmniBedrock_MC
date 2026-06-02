from __future__ import annotations

from backend.app.core.config import settings
from backend.app.core.security import verify_token
from backend.app.services.server_manager import ServerManager

server_manager = ServerManager()
