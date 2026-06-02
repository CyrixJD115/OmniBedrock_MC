import { writable } from 'svelte/store';

export type ToastType = 'success' | 'error' | 'info';

export interface ToastAction {
  label: string;
  callback: () => void;
}

export interface Toast {
  id: number;
  message: string;
  type: ToastType;
  action?: ToastAction;
  duration: number;
}

let nextId = 0;
export const toasts = writable<Toast[]>([]);

export function addToast(message: string, type: ToastType = 'info', duration = 4000, action?: ToastAction) {
  const id = nextId++;
  toasts.update(t => [...t, { id, message, type, action, duration }]);
  if (duration > 0) {
    setTimeout(() => {
      toasts.update(t => t.filter(toast => toast.id !== id));
    }, duration);
  }
}

export function dismissToast(id: number) {
  toasts.update(t => t.filter(toast => toast.id !== id));
}

export function success(msg: string) { addToast(msg, 'success'); }
export function error(msg: string) { addToast(msg, 'error', 6000); }
export function info(msg: string) { addToast(msg, 'info'); }
