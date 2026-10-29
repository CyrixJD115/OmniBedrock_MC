import { writable, derived } from 'svelte/store';

export interface UserInfo {
  username: string;
  role: string;
  display_name: string;
  created_at: string;
  last_login: string;
  permissions: string[];
  /** True while the seeded default password is still active (first run). */
  default_credentials?: boolean;
}

export const currentUser = writable<UserInfo | null>(null);
export const authToken = writable<string>('');

export const userPermissions = derived(currentUser, ($u) => $u?.permissions ?? []);
