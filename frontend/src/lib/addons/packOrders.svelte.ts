// World pack-order state with debounced autosave: owns the BP/RP activation
// order for the selected world, the dirty flag, and every order mutation
// (activate/deactivate/reorder/organize) shared by the Addon Organizer tabs.
// Constructed by the page, which supplies the current world/permission lookups
// and the action-log sink as callbacks.
import { addToast } from '$stores/toast';
import { api } from '$lib/api/client';
import { dateOf, kindOf, type PackKind } from '$lib/addons/display';
import type { Addon } from '$types/index';

/** Sentinel world value meaning "no specific world selected". */
export const ALL_WORLDS = '__all__';

const AUTOSAVE_DELAY_MS = 600;

export interface PackOrdersContext {
  log: (msg: string) => void;
  world: () => string;
  canManage: () => boolean;
}

export class PackOrders {
  orderBP = $state<string[]>([]);
  orderRP = $state<string[]>([]);
  orderDirty = $state(false);

  private saveTimer: ReturnType<typeof setTimeout> | undefined;
  private pendingSave: { world: string; bp: string[]; rp: string[] } | null = null;

  constructor(private ctx: PackOrdersContext) {}

  orderOf(t: PackKind): string[] {
    return t === 'bp' ? this.orderBP : this.orderRP;
  }

  setOrderOf(t: PackKind, v: string[]) {
    if (t === 'bp') this.orderBP = v;
    else this.orderRP = v;
  }

  isActive(p: Addon): boolean {
    return this.orderOf(kindOf(p)).includes(p.uuid);
  }

  /** Mark the order dirty and schedule the debounced autosave. */
  touchOrder() {
    const world = this.ctx.world();
    if (!this.ctx.canManage() || !world || world === ALL_WORLDS) return;
    this.orderDirty = true;
    this.pendingSave = { world, bp: this.orderBP, rp: this.orderRP };
    clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => this.flushPendingSave(), AUTOSAVE_DELAY_MS);
  }

  flushPendingSave() {
    clearTimeout(this.saveTimer);
    this.saveTimer = undefined;
    const p = this.pendingSave;
    this.pendingSave = null;
    if (p) void this.saveOrdersFor(p.world, p.bp, p.rp, true);
  }

  /** Drop any in-flight debounced edit (used on instance/world switches). */
  cancelPending() {
    clearTimeout(this.saveTimer);
    this.saveTimer = undefined;
    this.pendingSave = null;
  }

  async saveOrdersFor(world: string, bp: string[], rp: string[], auto: boolean) {
    try {
      await api.setPackOrder(world, 'behavior_packs', bp);
      await api.setPackOrder(world, 'resource_packs', rp);
      this.orderDirty = false;
      if (auto) this.ctx.log(`Pack order auto-saved (${world})`);
    } catch (e: any) {
      addToast(`Save failed: ${e.message}`, 'error');
      this.ctx.log(`Order save failed: ${e.message}`);
    }
  }

  async saveOrder() {
    const world = this.ctx.world();
    if (!world || world === ALL_WORLDS || !this.ctx.canManage()) return;
    clearTimeout(this.saveTimer);
    this.saveTimer = undefined;
    this.pendingSave = null;
    await this.saveOrdersFor(world, this.orderBP, this.orderRP, false);
    addToast('Pack order saved', 'success');
    this.ctx.log('Pack order saved');
  }

  activate(p: Addon) {
    if (!this.ctx.canManage()) return;
    const t = kindOf(p);
    const order = this.orderOf(t);
    if (!order.includes(p.uuid)) {
      this.setOrderOf(t, [...order, p.uuid]);
      this.touchOrder();
      this.ctx.log(`Activated ${p.name}`);
      addToast(`Activated ${p.name}`, 'success');
    }
  }

  deactivate(p: Addon) {
    if (!this.ctx.canManage()) return;
    const t = kindOf(p);
    if (this.orderOf(t).includes(p.uuid)) {
      this.setOrderOf(t, this.orderOf(t).filter((u) => u !== p.uuid));
      this.touchOrder();
      this.ctx.log(`Deactivated ${p.name}`);
      addToast(`Deactivated ${p.name}`, 'info');
    }
  }

  removeFromOrder(t: PackKind, uuid: string) {
    if (!this.ctx.canManage()) return;
    this.setOrderOf(t, this.orderOf(t).filter((u) => u !== uuid));
    this.touchOrder();
  }

  moveActive(packType: PackKind, uuid: string, dir: -1 | 1) {
    if (!this.ctx.canManage()) return;
    const order = [...this.orderOf(packType)];
    const i = order.indexOf(uuid);
    const j = i + dir;
    if (i < 0 || j < 0 || j >= order.length) return;
    [order[i], order[j]] = [order[j], order[i]];
    this.setOrderOf(packType, order);
    this.touchOrder();
  }

  deactivateAll() {
    if (!this.ctx.canManage()) return;
    const n = this.orderBP.length + this.orderRP.length;
    if (n === 0) return;
    this.orderBP = [];
    this.orderRP = [];
    this.touchOrder();
    addToast(`Deactivated ${n} pack(s)`, 'info');
    this.ctx.log('Deactivated all packs');
  }

  organizeActive(
    mode: 'az' | 'za' | 'newest' | 'oldest',
    packOf: (t: PackKind, uuid: string) => Addon | undefined,
  ) {
    if (!this.ctx.canManage()) return;
    for (const t of ['bp', 'rp'] as const) {
      const inOrder = this.orderOf(t)
        .map((u) => packOf(t, u))
        .filter((p): p is Addon => !!p);
      const missing = this.orderOf(t).filter((u) => !inOrder.some((p) => p.uuid === u));
      const sorted = [...inOrder].sort((a, b) => {
        if (mode === 'az') return a.name.localeCompare(b.name);
        if (mode === 'za') return b.name.localeCompare(a.name);
        const d = dateOf(a) - dateOf(b);
        return mode === 'newest' ? -d : d;
      });
      this.setOrderOf(t, [...sorted.map((p) => p.uuid), ...missing]);
    }
    this.touchOrder();
    const label = { az: 'Name A→Z', za: 'Name Z→A', newest: 'Newest first', oldest: 'Oldest first' }[mode];
    addToast(`Active lists re-sorted (${label})`, 'success');
    this.ctx.log(`Organized active lists: ${label}`);
  }
}
