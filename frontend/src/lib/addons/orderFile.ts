// Addon pack-order import/export: serializes the current order to a JSON
// download and opens a file picker that returns the raw imported text.
// Parsing and state application stay with the caller.

export interface ExportedAddonOrder {
  world: string;
  behavior_order: string[];
  resource_order: string[];
  hidden_uuids: string[];
  exported_at: string;
}

export interface PickedFile {
  text: string;
  fileName: string;
}

/** Trigger a JSON download of the given order snapshot. */
export function downloadOrderFile(data: ExportedAddonOrder): void {
  const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = `addon-order-${data.world}-${new Date().toISOString().slice(0, 10)}.json`;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

/**
 * Open a .json file picker and resolve with the file's text and name;
 * resolves null when the picker is dismissed. Read errors reject.
 */
export function pickOrderFile(): Promise<PickedFile | null> {
  return new Promise((resolve, reject) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.json';
    input.addEventListener('cancel', () => resolve(null));
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) {
        resolve(null);
        return;
      }
      try {
        resolve({ text: await file.text(), fileName: file.name });
      } catch (e) {
        reject(e);
      }
    };
    input.click();
  });
}
