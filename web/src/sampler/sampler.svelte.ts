// Whether the sample editor is showing, and for which oscillator.

class SampleEditorView {
  osc = $state<number | null>(null);

  open(osc: number): void {
    this.osc = osc;
  }

  close(): void {
    this.osc = null;
  }
}

export const sampleEditor = new SampleEditorView();
