// Web MIDI input (Chrome, Edge and Firefox; Safari has none, and the
// on-screen wheels and keyboard stand in). Every input port plays the synth,
// and ports plugged in later are picked up.

export type MidiStatus = 'unsupported' | 'off' | 'asking' | 'on' | 'denied';

export class WebMidi {
  status: MidiStatus;
  inputs: string[] = [];
  private access: MIDIAccess | null = null;
  private readonly subs = new Set<() => void>();

  constructor(private readonly onBytes: (bytes: Uint8Array, time: number) => void) {
    this.status = typeof navigator !== 'undefined' && 'requestMIDIAccess' in navigator ? 'off' : 'unsupported';
  }

  subscribe(fn: () => void): () => void {
    this.subs.add(fn);
    fn();
    return () => this.subs.delete(fn);
  }

  private emit(): void {
    for (const fn of this.subs) fn();
  }

  /** Turn MIDI on if the browser already allows it (no prompt). */
  async auto(): Promise<void> {
    if (this.status !== 'off') return;
    try {
      const p = await navigator.permissions.query({ name: 'midi' as PermissionName });
      if (p.state === 'granted') await this.enable();
    } catch {
      // permissions.query doesn't know 'midi' here: wait for the button
    }
  }

  /** Ask for MIDI access (may prompt) and listen to every input. */
  async enable(): Promise<void> {
    if (this.status === 'unsupported' || this.status === 'on' || this.status === 'asking') return;
    this.status = 'asking';
    this.emit();
    try {
      this.access = await navigator.requestMIDIAccess({ sysex: false });
    } catch {
      this.status = 'denied';
      this.emit();
      return;
    }
    this.access.onstatechange = () => this.bind();
    this.status = 'on';
    this.bind();
  }

  private bind(): void {
    if (!this.access) return;
    const names: string[] = [];
    for (const input of this.access.inputs.values()) {
      input.onmidimessage = (e) => e.data && this.onBytes(e.data, e.timeStamp);
      if (input.state === 'connected') names.push(input.name ?? 'MIDI input');
    }
    this.inputs = names;
    this.emit();
  }
}
