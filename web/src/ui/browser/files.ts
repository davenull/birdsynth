// Saving a file and opening one, for preset (and MIDI) export and import.

export function download(name: string, data: string | ArrayBuffer, type = 'application/json'): void {
  const url = URL.createObjectURL(new Blob([data], { type }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/** A safe file name for a preset. */
export function fileName(name: string): string {
  return `${(name.trim() || 'Untitled').replace(/[\\/:*?"<>|]+/g, '-')}.birdsynth.json`;
}

/** Ask for files (resolves with none if the dialog is dismissed). */
export function pick(accept: string, multiple = false): Promise<File[]> {
  return new Promise((ok) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.multiple = multiple;
    input.onchange = () => ok([...(input.files ?? [])]);
    input.oncancel = () => ok([]);
    input.click();
  });
}
