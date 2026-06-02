from __future__ import annotations

from pathlib import Path

from fastapi import APIRouter, Depends, HTTPException

from backend.app.core.config import settings
from backend.app.core.security import verify_token
from backend.app.models.user import User

router = APIRouter(prefix="/files", tags=["files"])


@router.get("/")
async def list_ini_files(_user: User = Depends(verify_token)) -> list[dict]:
    ini_dir = Path(settings.ini_dir)
    if not ini_dir.exists():
        return []
    files = []
    for f in sorted(ini_dir.iterdir()):
        if f.is_file() and f.suffix in (".ini", ".json", ".txt"):
            files.append({
                "name": f.name,
                "path": str(f),
                "size": f.stat().st_size,
                "modified": f.stat().st_mtime,
            })
    return files


@router.get("/{filename}")
async def read_file(filename: str, _user: User = Depends(verify_token)) -> dict:
    file_path = Path(settings.ini_dir) / filename
    if not file_path.exists() or not file_path.is_file():
        raise HTTPException(status_code=404, detail="File not found")
    return {
        "name": filename,
        "content": file_path.read_text(encoding="utf-8"),
    }


@router.put("/{filename}")
async def write_file(filename: str, body: dict, _user: User = Depends(verify_token)) -> dict:
    file_path = Path(settings.ini_dir) / filename
    content = body.get("content", "")
    file_path.parent.mkdir(parents=True, exist_ok=True)
    file_path.write_text(content, encoding="utf-8")
    return {"success": True, "message": f"Saved {filename}"}


@router.delete("/{filename}")
async def delete_file(filename: str, _user: User = Depends(verify_token)) -> dict:
    file_path = Path(settings.ini_dir) / filename
    if not file_path.exists():
        raise HTTPException(status_code=404, detail="File not found")
    file_path.unlink()
    return {"success": True, "message": f"Deleted {filename}"}
