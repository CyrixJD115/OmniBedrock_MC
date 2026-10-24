from __future__ import annotations

import json
import uuid as uuid_lib
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

    def _write_manifest(self, path: str | Path, manifest: dict) -> bool:
        manifest_path = Path(path) / "manifest.json"
        try:
            manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
            return True
        except OSError:
            return False

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
                            "manifest": manifest,
                        }
                        if key == "behavior_packs":
                            behavior_packs.append(info)
                        else:
                            resource_packs.append(info)

        return {"behavior_packs": behavior_packs, "resource_packs": resource_packs}

    def get_manifest(self, path: str) -> dict | None:
        return self._read_manifest(Path(path))

    def update_manifest(self, path: str, manifest: dict) -> bool:
        return self._write_manifest(path, manifest)

    def rename_addon(self, path: str, new_name: str) -> tuple[bool, str]:
        """Rename an addon folder on disk."""
        old_path = Path(path)
        if not old_path.exists() or not old_path.is_dir():
            return False, "Addon path not found"
        new_path = old_path.parent / new_name
        if new_path.exists():
            return False, "An addon with that name already exists"
        try:
            old_path.rename(new_path)
            return True, str(new_path)
        except OSError as e:
            return False, str(e)

    def randomize_uuid(self, path: str) -> tuple[bool, str | None]:
        """Generate a new random UUID for the addon's manifest."""
        manifest = self._read_manifest(Path(path))
        if not manifest:
            return False, None
        new_uuid = str(uuid_lib.uuid4())
        if "header" not in manifest:
            manifest["header"] = {}
        manifest["header"]["uuid"] = new_uuid
        ok = self._write_manifest(path, manifest)
        return ok, new_uuid if ok else None

    def change_uuid(self, path: str, new_uuid: str) -> bool:
        """Set a specific UUID in the addon's manifest."""
        manifest = self._read_manifest(Path(path))
        if not manifest:
            return False
        if "header" not in manifest:
            manifest["header"] = {}
        manifest["header"]["uuid"] = new_uuid
        return self._write_manifest(path, manifest)

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
