from __future__ import annotations

import json
from pathlib import Path

from backend.app.core.config import settings


class AddonService:
    def __init__(self) -> None:
        self._worlds_dir = Path(settings.bedrock_server_dir) / "worlds"
        self._worlds_dir.mkdir(parents=True, exist_ok=True)

    def _get_worlds(self) -> list[Path]:
        if not self._worlds_dir.exists():
            return []
        return [d for d in self._worlds_dir.iterdir() if d.is_dir()]

    def _read_manifest(self, pack_path: Path) -> dict | None:
        manifest_path = pack_path / "manifest.json"
        if manifest_path.exists():
            try:
                return json.loads(manifest_path.read_text(encoding="utf-8"))
            except (json.JSONDecodeError, OSError):
                return None
        return None

    def list_addons(self) -> dict:
        behavior_packs: list[dict] = []
        resource_packs: list[dict] = []

        for world in self._get_worlds():
            for pack_type, key in [("behavior_packs", "behavior_packs"), ("resource_packs", "resource_packs")]:
                packs_dir = world / pack_type
                if not packs_dir.exists():
                    continue
                for pack in packs_dir.iterdir():
                    if pack.is_dir():
                        manifest = self._read_manifest(pack)
                        info = {
                            "name": pack.name,
                            "path": str(pack),
                            "world": world.name,
                            "pack_type": pack_type,
                            "uuid": (manifest.get("header", {}).get("uuid", "") if manifest else ""),
                            "version": (manifest.get("header", {}).get("version", []) if manifest else []),
                            "valid": manifest is not None,
                        }
                        if key == "behavior_packs":
                            behavior_packs.append(info)
                        else:
                            resource_packs.append(info)

        return {"behavior_packs": behavior_packs, "resource_packs": resource_packs}

    def get_manifest(self, path: str) -> dict | None:
        return self._read_manifest(Path(path))

    def update_manifest(self, path: str, manifest: dict) -> bool:
        manifest_path = Path(path) / "manifest.json"
        try:
            manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
            return True
        except OSError:
            return False

    def get_pack_order(self, world: str, pack_type: str) -> list[dict]:
        world_path = self._worlds_dir / world
        order_file = world_path / f"world_{pack_type}.json"
        if order_file.exists():
            try:
                return json.loads(order_file.read_text(encoding="utf-8"))
            except (json.JSONDecodeError, OSError):
                return []
        return []

    def set_pack_order(self, world: str, pack_type: str, uuids: list[str]) -> bool:
        world_path = self._worlds_dir / world
        order_file = world_path / f"world_{pack_type}.json"
        order = [{"pack_id": uid, "version": [1, 0, 0]} for uid in uuids]
        try:
            order_file.write_text(json.dumps(order, indent=2), encoding="utf-8")
            return True
        except OSError:
            return False
