import { writable, derived } from 'svelte/store';

export interface UserInfo {
  username: string;
  role: string;
  display_name: string;
  created_at: string;
  last_login: string;
  permissions: string[];
}

export const currentUser = writable<UserInfo | null>(null);
export const authToken = writable<string>('');

export const userPermissions = derived(currentUser, ($u) => $u?.permissions ?? []);
