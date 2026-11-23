// Players page data (GET /players, GET /players/known).

export type PermissionLevel = 'visitor' | 'member' | 'operator';

export interface OnlinePlayer {
  name: string;
  xuid: string | null;
  online_since: number | null;
  permission: PermissionLevel;
}

export interface KnownPlayer {
  name: string;
  xuid: string | null;
  first_seen: number;
  last_seen: number;
  sessions: number;
  playtime_secs: number;
  online_since: number | null;
  permission: PermissionLevel;
  allowlisted: boolean;
  banned: boolean;
}

export interface PlayersOverview {
  players: KnownPlayer[];
  allowlist: { name: string; xuid?: string; ignoresPlayerLimit: boolean }[];
  operators: { xuid: string; permission: string; name: string | null }[];
  bans: { name: string; reason: string | null; created: string | null; source: string | null; expires: string | null }[];
  allowlist_enabled: boolean | null;
  max_players: number | null;
  default_permission: string | null;
  running: boolean;
  bans_supported: boolean;
}

export type PlayerAction =
  | 'kick' | 'tell' | 'ban' | 'pardon' | 'op' | 'deop' | 'permission'
  | 'allowlist_add' | 'allowlist_remove' | 'forget';
