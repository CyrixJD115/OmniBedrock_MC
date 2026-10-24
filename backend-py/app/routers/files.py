from __future__ import annotations

from pathlib import Path

from fastapi import APIRouter, Depends, HTTPException

from backend.app.core.config import settings
from backend.app.core.permissions import FILES_EDIT
from backend.app.core.security import require_permission, verify_token
from backend.app.models.user import User

router = APIRouter(prefix="/files", tags=["files"])

DATA_DIR = Path(settings.data_dir)
SENSITIVE = {"users.yaml", "console_lock_state.yaml"}
ALLOWED_EXTENSIONS = {".ini", ".yaml", ".yml", ".json", ".txt"}


def _safe_path(filename: str) -> Path:
    path = (DATA_DIR / filename).resolve()
    if not str(path).startswith(str(DATA_DIR.resolve())):
        raise HTTPException(status_code=400, detail="Invalid path")
    return path


@router.get("/")
async def list_files(_user: User = Depends(verify_token)) -> list[dict]:
    files = []
    for entry in sorted(DATA_DIR.iterdir(), key=lambda e: e.name):
        if entry.name in SENSITIVE:
            continue
        if entry.suffix.lower() not in ALLOWED_EXTENSIONS:
            continue
        if not entry.is_file():
            continue
        stat = entry.stat()
        files.append({
            "name": entry.name,
            "path": str(entry),
            "size": stat.st_size,
            "modified": int(stat.st_mtime),
        })
    return files


@router.get("/{filename}")
async def read_file(filename: str, _user: User = Depends(verify_token)) -> dict:
    path = _safe_path(filename)
    if path.name in SENSITIVE:
        raise HTTPException(status_code=403, detail="Access denied")
    if not path.is_file():
        raise HTTPException(status_code=404, detail="File not found")
    return {"name": path.name, "content": path.read_text(encoding="utf-8")}


@router.put("/{filename}")
async def write_file(filename: str, body: dict, _user: User = Depends(require_permission(FILES_EDIT))):
    path = _safe_path(filename)
    if path.name in SENSITIVE:
        raise HTTPException(status_code=403, detail="Access denied")
    path.write_text(body.get("content", ""), encoding="utf-8")
    return {"success": True}


@router.delete("/{filename}")
async def delete_file(filename: str, _user: User = Depends(require_permission(FILES_EDIT))):
    path = _safe_path(filename)
    if path.name in SENSITIVE:
        raise HTTPException(status_code=403, detail="Access denied")
    if not path.is_file():
        raise HTTPException(status_code=404, detail="File not found")
    path.unlink()
    return {"success": True}
