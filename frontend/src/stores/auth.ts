import { writable } from 'svelte/store';

export interface UserInfo {
  username: string;
  role: 'owner' | 'admin' | 'moderator' | 'viewer';
  display_name: string;
  created_at: string;
  last_login: string;
}

export const currentUser = writable<UserInfo | null>(null);
export const authToken = writable<string>('');
