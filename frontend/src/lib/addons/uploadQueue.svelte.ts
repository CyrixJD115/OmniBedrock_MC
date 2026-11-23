// Addon upload queue: validates files up front, uploads one at a time with
// progress, and keeps each file's outcome visible (not just a toast) so a
// batch of drops can be reviewed and freshly installed packs activated.
import { api } from '$lib/api/client';

export const MAX_UPLOAD_BYTES = 512 * 1024 * 1024;
const ACCEPTED = /\.(mcpack|mcaddon|zip)$/i;

export type UploadState = 'queued' | 'uploading' | 'installed' | 'failed' | 'rejected';

export interface InstalledPack {
  name: string;
  pack_type: string;
  uuid: string;
  version: number[];
}

export interface UploadItem {
  id: number;
  name: string;
  size: number;
  world: string;
  state: UploadState;
  progress: number;
  message: string;
  installed: InstalledPack[];
}

let seq = 0;

export class UploadQueue {
  items = $state<UploadItem[]>([]);
  busy = $state(false);
  private pending: { item: UploadItem; file: File }[] = [];

  /** Queue files for `world`; invalid ones are listed as rejected. */
  add(files: File[], world: string, onDone: () => void) {
    const fresh: { item: UploadItem; file: File }[] = [];
    for (const file of files) {
      const item: UploadItem = { id: ++seq, name: file.name, size: file.size, world, state: 'queued', progress: 0, message: '', installed: [] };
      if (!ACCEPTED.test(file.name)) {
        item.state = 'rejected';
        item.message = 'Only .mcpack, .mcaddon and .zip files are supported';
      } else if (file.size > MAX_UPLOAD_BYTES) {
        item.state = 'rejected';
        item.message = 'Larger than the 512 MiB limit';
      } else {
        fresh.push({ item, file });
      }
      this.items.push(item);
    }
    if (fresh.length) void this.drain(fresh, onDone);
  }

  private async drain(jobs: { item: UploadItem; file: File }[], onDone: () => void) {
    // Another batch is running: it picks these up after its own files.
    this.pending.push(...jobs);
    if (this.busy) return;
    this.busy = true;
    let installedAny = false;
    while (this.pending.length) {
      const { item, file } = this.pending.shift()!;
      const live = this.items.find((i) => i.id === item.id);
      if (!live) continue;
      live.state = 'uploading';
      try {
        const r = await api.uploadAddonWithProgress(live.world, file, (f) => (live.progress = f));
        live.state = 'installed';
        live.progress = 1;
        live.installed = r.installed ?? [];
        live.message = r.message;
        installedAny = true;
      } catch (e: any) {
        live.state = 'failed';
        live.message = e.message;
      }
    }
    this.busy = false;
    if (installedAny) onDone();
  }

  dismiss(id: number) {
    this.items = this.items.filter((i) => i.id !== id || i.state === 'uploading' || i.state === 'queued');
  }

  clearFinished() {
    this.items = this.items.filter((i) => i.state === 'uploading' || i.state === 'queued');
  }
}
