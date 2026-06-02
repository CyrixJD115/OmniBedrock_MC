from __future__ import annotations

from pathlib import Path

from fastapi import APIRouter, Depends, HTTPException

from backend.app.core.config import settings
from backend.app.core.security import verify_token

router = APIRouter(prefix="/worlds", tags=["worlds"])


@router.get("/")
async def list_worlds(auth: str = Depends(verify_token)) -> list[str]:
    worlds_dir = Path(settings.bedrock_server_dir) / "worlds"
    if not worlds_dir.exists():
        return []
    return [d.name for d in worlds_dir.iterdir() if d.is_dir()]


@router.get("/{world}")
async def get_world_info(world: str, auth: str = Depends(verify_token)) -> dict:
    world_path = Path(settings.bedrock_server_dir) / "worlds" / world
    if not world_path.exists():
        raise HTTPException(status_code=404, detail="World not found")
    info = {"name": world, "path": str(world_path), "size_bytes": 0, "files": 0}
    total_size = 0
    total_files = 0
    for f in world_path.rglob("*"):
        if f.is_file():
            total_size += f.stat().st_size
            total_files += 1
    info["size_bytes"] = total_size
    info["files"] = total_files
    return info


@router.get("/{world}/contents")
async def get_world_contents(world: str, auth: str = Depends(verify_token)) -> dict:
    world_path = Path(settings.bedrock_server_dir) / "worlds" / world
    if not world_path.exists():
        raise HTTPException(status_code=404, detail="World not found")

    def _build_tree(p: Path) -> dict | list:
        if p.is_dir():
            children = []
            for child in sorted(p.iterdir()):
                children.append(_build_tree(child))
            return {"name": p.name, "type": "directory", "children": children}
        return {"name": p.name, "type": "file", "size": p.stat().st_size}

    return _build_tree(world_path)
