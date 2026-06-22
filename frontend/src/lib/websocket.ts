
function getWsBase(): string {
  if (typeof location === 'undefined') return 'ws://localhost:17754';
  return `${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}`;
}

type MessageHandler = (data: Record<string, unknown>) => void;

class WebSocketManager {
  private connections: Map<string, { ws: WebSocket; handlers: Set<MessageHandler>; reconnectTimer?: ReturnType<typeof setTimeout>; closed: boolean }> = new Map();

  connect(path: string, onMessage: MessageHandler): () => void {
    const key = path;

    if (!this.connections.has(key)) {
      const wsBase = getWsBase();
      const url = `${wsBase}${path}`;
      const ws = new WebSocket(url);
      const handlers = new Set<MessageHandler>([onMessage]);
      const entry: { ws: WebSocket; handlers: Set<MessageHandler>; reconnectTimer?: ReturnType<typeof setTimeout>; closed: boolean } = { ws, handlers, closed: false };

      ws.onopen = () => {
        if (entry.reconnectTimer) {
          clearTimeout(entry.reconnectTimer);
          entry.reconnectTimer = undefined;
        }
      };

      ws.onmessage = (event) => {
        try {
          const data = JSON.parse(event.data);
          if (data.type === 'ping') return;
          for (const handler of entry.handlers) {
            handler(data);
          }
        } catch { /* ignore */ }
      };

      ws.onclose = () => {
        if (entry.closed) return;
        entry.reconnectTimer = setTimeout(() => {
          this.connections.delete(key);
          for (const h of handlers) {
            this.connect(path, h);
          }
        }, 3000);
      };

      this.connections.set(key, entry);
    } else {
      const entry = this.connections.get(key)!;
      entry.handlers.add(onMessage);
    }

    return () => {
      const entry = this.connections.get(key);
      if (entry) {
        entry.handlers.delete(onMessage);
        if (entry.handlers.size === 0) {
          entry.closed = true;
          entry.ws.close();
          if (entry.reconnectTimer) clearTimeout(entry.reconnectTimer);
          this.connections.delete(key);
        }
      }
    };
  }

  disconnect(path: string) {
    const entry = this.connections.get(path);
    if (entry) {
      entry.closed = true;
      entry.ws.close();
      if (entry.reconnectTimer) clearTimeout(entry.reconnectTimer);
      this.connections.delete(path);
    }
  }
}

export const wsManager = new WebSocketManager();
