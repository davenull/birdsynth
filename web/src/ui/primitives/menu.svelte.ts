// One context menu for the whole app (knobs and anything else that wants
// one): show it at the pointer with a list of actions.

export interface MenuItem {
  label: string;
  action: () => void;
  disabled?: boolean;
}

class Menu {
  open = $state(false);
  x = $state(0);
  y = $state(0);
  items = $state<MenuItem[]>([]);

  show(e: MouseEvent, items: MenuItem[]): void {
    e.preventDefault();
    this.items = items;
    this.x = e.clientX;
    this.y = e.clientY;
    this.open = true;
  }

  close(): void {
    this.open = false;
  }
}

export const menu = new Menu();
