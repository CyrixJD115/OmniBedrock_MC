/**
 * Instance create/edit form models shared by the instances page and its
 * modals, plus the bind-mount textarea parser.
 */
import type { AccessEntry } from '$types/index';

export interface CreateInstanceForm {
  name: string;
  backend: 'vanilla' | 'endstone';
  deployment: string;
  dockerImage: string;
  dockerMounts: string;
  memory: string | number;
  cpus: string | number;
  port: string | number;
  vanillaVersion: string;
  endstoneVersion: string;
  installAfter: boolean;
  autostart: boolean;
}

export interface EditInstanceForm {
  name: string;
  port: string | number;
  memory: string | number;
  cpus: string | number;
  deployment: string;
  dockerImage: string;
  dockerMounts: string;
  autostart: boolean;
  access: AccessEntry[];
}

/** Parse the one-per-line "host:container[:opts]" textarea into a mount list. */
export function parseMounts(raw: string): string[] {
  return raw
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
}
