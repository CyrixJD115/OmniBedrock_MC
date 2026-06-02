from __future__ import annotations

import asyncio

from fastapi import APIRouter, Depends, HTTPException

from backend.app.core.dependencies import server_manager
from backend.app.core.security import verify_token
from backend.app.models.user import User
from backend.app.schemas.player import PlayerActionRequest, PlayerListResponse

router = APIRouter(prefix="/players", tags=["players"])


@router.get("/")
async def list_players(_user: User = Depends(verify_token)) -> PlayerListResponse:
    if server_manager.status.value != "running":
        return PlayerListResponse(players=[], count=0)

    players: list[dict] = []
    result_q = server_manager.subscribe_stdout()

    try:
        await server_manager.send_command("list")

        try:
            line = await asyncio.wait_for(result_q.get(), timeout=3)
            if "players online" in line.lower():
                parts = line.split(":")
                if len(parts) > 1:
                    names = [n.strip() for n in parts[-1].split(",") if n.strip()]
                    players = [{"name": n} for n in names]
        except asyncio.TimeoutError:
            pass
    finally:
        server_manager.unsubscribe_stdout(result_q)

    return PlayerListResponse(players=players, count=len(players))


@router.post("/action")
async def player_action(req: PlayerActionRequest, _user: User = Depends(verify_token)) -> dict:
    valid_actions = ["kick", "ban", "pardon", "op", "deop"]
    if req.action not in valid_actions:
        raise HTTPException(status_code=400, detail=f"Invalid action. Must be one of: {valid_actions}")

    if req.action == "kick" and req.reason:
        cmd = f"kick {req.target} {req.reason}"
    else:
        cmd = f"{req.action} {req.target}"

    await server_manager.send_command(cmd)
    return {"success": True, "message": f"Sent '{cmd}' to server"}
