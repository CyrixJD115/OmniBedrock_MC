// Pointer-based list sorting (mouse, pen and touch). Rows never move while
// dragging: an absolutely positioned indicator marks the insertion slot, so
// the row under the pointer cannot shift and flip the target back and forth.
// The slot is decided by row midpoints (above half → before, below → after),
// which also makes "move down by one" possible. The container auto-scrolls
// near its edges; Escape cancels; a click right after a drag is swallowed.

const THRESHOLD_PX = 5;
const EDGE_PX = 36;
const MAX_SCROLL_STEP = 14;

export interface SortCommit {
  order: string[];
  id: string;
  from: number;
  to: number;
}

export class Sortable {
  /** Id of the row being dragged, or null. */
  dragging = $state<string | null>(null);
  /** Insertion slot 0..n while dragging. */
  slot = $state<number | null>(null);
  /** Indicator offset (px) inside the scroll container. */
  indicatorTop = $state(0);

  private container: HTMLElement | null = null;
  private ids: string[] = [];
  private id = '';
  private startY = 0;
  private pointerY = 0;
  private active = false;
  private raf = 0;
  private commit: ((c: SortCommit) => void) | null = null;

  /** Call from a row's onpointerdown. `ids` is the current order. */
  start(e: PointerEvent, id: string, container: HTMLElement | null, ids: string[], commit: (c: SortCommit) => void) {
    if (e.button !== 0 || !container) return;
    const target = e.target as HTMLElement;
    // Row controls opt out with data-no-drag; a row's main (preview) button
    // stays draggable: the movement threshold keeps a plain click a click.
    if (target.closest('a, input, select, textarea, [data-no-drag]')) return;
    this.container = container;
    this.ids = ids;
    this.id = id;
    this.startY = this.pointerY = e.clientY;
    this.active = false;
    this.commit = commit;
    window.addEventListener('pointermove', this.move, { passive: false });
    window.addEventListener('pointerup', this.up);
    window.addEventListener('pointercancel', this.cancel);
    window.addEventListener('keydown', this.key);
  }

  private begin() {
    this.active = true;
    this.dragging = this.id;
    document.body.style.userSelect = 'none';
    document.body.style.cursor = 'grabbing';
    const tick = () => {
      this.autoscroll();
      this.raf = requestAnimationFrame(tick);
    };
    this.raf = requestAnimationFrame(tick);
  }

  private rows(): HTMLElement[] {
    return this.container ? [...this.container.querySelectorAll<HTMLElement>('[data-sort-id]')] : [];
  }

  private locate() {
    const rows = this.rows();
    if (!this.container || rows.length === 0) return;
    let slot = rows.length;
    for (let i = 0; i < rows.length; i++) {
      const r = rows[i].getBoundingClientRect();
      if (this.pointerY < r.top + r.height / 2) {
        slot = i;
        break;
      }
    }
    this.slot = slot;
    const edge = slot < rows.length ? rows[slot].offsetTop - 3 : rows[rows.length - 1].offsetTop + rows[rows.length - 1].offsetHeight + 1;
    this.indicatorTop = edge;
  }

  private autoscroll() {
    const c = this.container;
    if (!c) return;
    const r = c.getBoundingClientRect();
    let step = 0;
    if (this.pointerY < r.top + EDGE_PX) step = -Math.ceil(((r.top + EDGE_PX - this.pointerY) / EDGE_PX) * MAX_SCROLL_STEP);
    else if (this.pointerY > r.bottom - EDGE_PX) step = Math.ceil(((this.pointerY - (r.bottom - EDGE_PX)) / EDGE_PX) * MAX_SCROLL_STEP);
    if (step) {
      c.scrollTop += Math.max(-MAX_SCROLL_STEP, Math.min(MAX_SCROLL_STEP, step));
      this.locate();
    }
  }

  private move = (e: PointerEvent) => {
    this.pointerY = e.clientY;
    if (!this.active) {
      if (Math.abs(e.clientY - this.startY) < THRESHOLD_PX) return;
      this.begin();
    }
    e.preventDefault();
    this.locate();
  };

  private up = () => {
    const wasActive = this.active;
    const slot = this.slot;
    const commit = this.commit;
    const id = this.id;
    const ids = this.ids;
    this.stop();
    if (!wasActive) return;
    // Swallow the click the browser fires after a drag that ended on a row.
    window.addEventListener('click', swallow, { capture: true, once: true });
    setTimeout(() => window.removeEventListener('click', swallow, { capture: true }), 0);
    if (slot === null || !commit) return;
    const from = ids.indexOf(id);
    let to = slot > from ? slot - 1 : slot;
    to = Math.max(0, Math.min(ids.length - 1, to));
    if (from < 0 || to === from) return;
    const order = ids.filter((x) => x !== id);
    order.splice(to, 0, id);
    commit({ order, id, from, to });
  };

  private cancel = () => this.stop();

  private key = (e: KeyboardEvent) => {
    if (e.key === 'Escape') this.stop();
  };

  private stop() {
    cancelAnimationFrame(this.raf);
    window.removeEventListener('pointermove', this.move);
    window.removeEventListener('pointerup', this.up);
    window.removeEventListener('pointercancel', this.cancel);
    window.removeEventListener('keydown', this.key);
    if (this.active) {
      document.body.style.userSelect = '';
      document.body.style.cursor = '';
    }
    this.active = false;
    this.dragging = null;
    this.slot = null;
    this.commit = null;
  }
}

function swallow(e: Event) {
  e.stopPropagation();
  e.preventDefault();
}
