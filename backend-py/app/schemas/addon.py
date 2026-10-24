from __future__ import annotations

from pydantic import BaseModel


class AddonListResponse(BaseModel):
    behavior_packs: list[dict]
    resource_packs: list[dict]


class AddonReorderRequest(BaseModel):
    pack_type: str
    uuids: list[str]


class ManifestUpdateRequest(BaseModel):
    path: str
    manifest: dict


class RenameAddonRequest(BaseModel):
    path: str
    new_name: str


class ChangeUuidRequest(BaseModel):
    path: str
    uuid: str


class RandomizeUuidRequest(BaseModel):
    path: str
