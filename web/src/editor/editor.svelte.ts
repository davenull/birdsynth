// Whether the wavetable editor is showing, and for which oscillator.

import type { TableStore } from '../state/tables';
import { WtEditor } from './model';

class EditorView {
  /** The oscillator being edited, or null when the editor is closed. */
  osc = $state<number | null>(null);
  private model: WtEditor | null = null;

  editor(tables: TableStore): WtEditor {
    return (this.model ??= new WtEditor(tables));
  }

  open(tables: TableStore, osc: number): void {
    this.editor(tables).open(osc);
    this.osc = osc;
  }

  close(): void {
    this.osc = null;
  }
}

export const editorView = new EditorView();
