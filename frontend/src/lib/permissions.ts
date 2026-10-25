/**
 * The permission catalog: display labels and editor grouping. Mirrors
 * auth::PERMISSIONS on the backend — the backend list remains the authority.
 */
import { ALL_PERMISSIONS, INSTANCE_PERMISSIONS } from '$types/index';

export interface PermMeta {
  perm: string;
  label: string;
  hint: string;
}

export const PERM_GROUPS: { label: string; hint: string; perms: PermMeta[] }[] = [
  {
    label: 'Console & runtime',
    hint: 'Live console and server lifecycle on instances this role can reach',
    perms: [
      { perm: 'CONSOLE_VIEW', label: 'View console', hint: 'Read the live server console' },
      { perm: 'CONSOLE_SEND', label: 'Send commands', hint: 'Type commands into the console' },
      { perm: 'SERVER_VIEW', label: 'View status', hint: 'See status, metrics and versions' },
      { perm: 'SERVER_MANAGE', label: 'Start / stop / configure', hint: 'Start, stop, restart, switch backend, edit runtime settings' },
    ],
  },
  {
    label: 'World content',
    hint: 'Worlds, properties, addons and the CurseForge store',
    perms: [
      { perm: 'PROPERTIES_VIEW', label: 'View properties', hint: 'Read server.properties' },
      { perm: 'PROPERTIES_EDIT', label: 'Edit properties', hint: 'Change server.properties values' },
      { perm: 'ADDONS_VIEW', label: 'View addons', hint: 'Browse behavior and resource packs' },
      { perm: 'ADDONS_MANAGE', label: 'Manage addons', hint: 'Upload, reorder and delete packs' },
      { perm: 'STORE_VIEW', label: 'Browse store', hint: 'Search the CurseForge store' },
      { perm: 'STORE_INSTALL', label: 'Install from store', hint: 'Install packs into worlds' },
      { perm: 'FILES_VIEW', label: 'View files', hint: 'Read server config files' },
      { perm: 'FILES_EDIT', label: 'Edit files', hint: 'Write server config files' },
    ],
  },
  {
    label: 'Players & backups',
    hint: 'Player moderation and world protection',
    perms: [
      { perm: 'PLAYERS_VIEW', label: 'View players', hint: 'See the player list' },
      { perm: 'PLAYERS_KICK', label: 'Kick players', hint: 'Kick players from the server' },
      { perm: 'PLAYERS_BAN', label: 'Ban players', hint: 'Ban and pardon players' },
      { perm: 'PLAYERS_OP', label: 'Op players', hint: 'Grant in-game operator status' },
      { perm: 'BACKUPS_VIEW', label: 'View backups', hint: 'List backups and history' },
      { perm: 'BACKUPS_CREATE', label: 'Create backups', hint: 'Run backups on demand' },
      { perm: 'BACKUPS_RESTORE', label: 'Restore backups', hint: 'Restore worlds from backups' },
      { perm: 'BACKUPS_DELETE', label: 'Delete backups', hint: 'Delete backup archives' },
    ],
  },
  {
    label: 'Instances',
    hint: 'Multi-instance management. Sharing and deletion also work per-instance',
    perms: [
      { perm: 'INSTANCES_CREATE', label: 'Create instances', hint: 'Create new servers (global only; quotas still apply)' },
      { perm: 'INSTANCES_VIEW_ALL', label: 'See all instances', hint: 'Discover and reach every instance (admin-level)' },
      { perm: 'INSTANCES_SHARE', label: 'Manage shares', hint: 'Grant and revoke instance access (also grantable per-instance)' },
      { perm: 'INSTANCES_DELETE', label: 'Delete instances', hint: 'Delete instances they can reach (also grantable per-instance)' },
      { perm: 'INSTANCES_LIMITS', label: 'Change resource limits', hint: 'Tune RAM/CPU/deployment/port (also grantable per-instance); quotas still apply' },
    ],
  },
  {
    label: 'Users & roles',
    hint: 'Account management. Delegation is bounded: you cannot grant what you do not hold',
    perms: [
      { perm: 'USERS_VIEW', label: 'View users', hint: 'See the user list' },
      { perm: 'USERS_CREATE', label: 'Create users', hint: 'Add accounts' },
      { perm: 'USERS_EDIT', label: 'Edit users', hint: 'Change names, passwords, roles and quotas*' },
      { perm: 'USERS_DELETE', label: 'Delete users', hint: 'Remove accounts' },
      { perm: 'ROLES_MANAGE', label: 'Manage roles', hint: 'Create, edit and delete roles' },
    ],
  },
  {
    label: 'Panel',
    hint: 'Panel-level views and settings',
    perms: [
      { perm: 'BROADCASTER_VIEW', label: 'View broadcaster', hint: 'See MCXboxBroadcast status' },
      { perm: 'BROADCASTER_MANAGE', label: 'Manage broadcaster', hint: 'Control the broadcaster' },
      { perm: 'AUDIT_VIEW', label: 'View audit log', hint: 'Read the audit trail' },
      { perm: 'SETTINGS_VIEW', label: 'View settings', hint: 'See panel settings' },
      { perm: 'SETTINGS_EDIT', label: 'Edit settings', hint: 'Change panel settings' },
    ],
  },
];

export const PERM_LABEL: Record<string, string> = Object.fromEntries(
  PERM_GROUPS.flatMap((g) => g.perms.map((p) => [p.perm, p.label]))
);

export function isInstancePermission(perm: string): boolean {
  return (INSTANCE_PERMISSIONS as readonly string[]).includes(perm);
}

export function allKnownPermissions(): string[] {
  return [...ALL_PERMISSIONS];
}

export function hexColor(color: string): string {
  const hex = (color || '').replace('#', '');
  return /^[0-9a-fA-F]{6}$/.test(hex) ? `#${hex}` : '#99AAB5';
}

/** A Discord-flavored preset palette for role colors. */
export const COLOR_PRESETS = [
  '5865F2', '57F287', 'FEE75C', 'EB459E', 'ED4245',
  'F0B232', '3498DB', '2ECC71', '9B59B6', '99AAB5',
];
