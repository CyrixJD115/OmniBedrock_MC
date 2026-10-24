import { getToken } from '$lib/api/client';

type MessageHandler = (data: Record<string, unknown>) => void;
interface Connection {
  path: string;
  ws?: WebSocket;
  handlers: Set<MessageHandler>;
  reconnectTimer?: ReturnType<typeof setTimeout>;
  watchdog?: ReturnType<typeof setInterval>;
  closed: boolean;
  attempts: number;
  lastMessage: number;
  sequence?: number;
  streamId?: string;
}

class WebSocketManager {
  private connections = new Map<string, Connection>();

  connect(path: string, onMessage: MessageHandler): () => void {
    const key = path.split('?')[0];
    let entry = this.connections.get(key);
    if (!entry) {
      entry = { path, handlers: new Set(), closed: false, attempts: 0, lastMessage: Date.now() };
      this.connections.set(key, entry);
    }
    entry.handlers.add(onMessage);
    if (!entry.ws && !entry.reconnectTimer) this.open(key, entry);
    const subscription = entry;
    return () => {
      subscription.handlers.delete(onMessage);
      if (subscription.handlers.size === 0) this.close(key, subscription);
    };
  }

  private open(key: string, entry: Connection) {
    if (entry.closed || this.connections.get(key) !== entry) return;
    const base = typeof location === 'undefined' ? 'http://localhost:17754' : location.origin;
    const url = new URL(entry.path, base);
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
    const token = getToken();
    if (token) url.searchParams.set('token', token);
    if (entry.sequence !== undefined && entry.streamId) {
      url.searchParams.set('after', String(entry.sequence));
      url.searchParams.set('stream_id', entry.streamId);
    }
    const ws = new WebSocket(url);
    entry.ws = ws;
    entry.lastMessage = Date.now();
    ws.onopen = () => { entry.attempts = 0; entry.lastMessage = Date.now(); };
    entry.watchdog = setInterval(() => {
      if (Date.now() - entry.lastMessage > 90_000) ws.close();
    }, 15_000);
    ws.onmessage = (event) => {
      entry.lastMessage = Date.now();
      let data: Record<string, unknown>;
      try { data = JSON.parse(event.data); } catch { return; }
      if (data.type === 'ping') return;
      if (data.type === 'hello' && typeof data.stream_id === 'string') {
        if (entry.streamId !== data.stream_id || data.resumed === false) entry.sequence = undefined;
        entry.streamId = data.stream_id;
      }
      if (data.type === 'console' && typeof data.sequence === 'number' && typeof data.stream_id === 'string') {
        if (entry.streamId === data.stream_id && entry.sequence !== undefined && data.sequence <= entry.sequence) return;
        entry.streamId = data.stream_id;
        entry.sequence = data.sequence;
      }
      for (const handler of entry.handlers) {
        try { handler(data); } catch (error) { console.error('WebSocket message handler failed', error); }
      }
    };
    ws.onclose = (event) => {
      if (entry.watchdog) clearInterval(entry.watchdog);
      entry.watchdog = undefined;
      entry.ws = undefined;
      if (entry.closed || event.code === 1008 || this.connections.get(key) !== entry) return;
      const delay = Math.min(30_000, 500 * 2 ** Math.min(entry.attempts++, 6)) + Math.random() * 250;
      entry.reconnectTimer = setTimeout(() => {
        entry.reconnectTimer = undefined;
        this.open(key, entry);
      }, delay);
    };
  }

  private close(key: string, entry: Connection) {
    entry.closed = true;
    if (entry.reconnectTimer) clearTimeout(entry.reconnectTimer);
    if (entry.watchdog) clearInterval(entry.watchdog);
    entry.ws?.close();
    if (this.connections.get(key) === entry) this.connections.delete(key);
  }

  disconnect(path: string) {
    const key = path.split('?')[0];
    const entry = this.connections.get(key);
    if (entry) this.close(key, entry);
  }

  disconnectAll() {
    for (const [key, entry] of this.connections) this.close(key, entry);
  }
}

export const wsManager = new WebSocketManager();
