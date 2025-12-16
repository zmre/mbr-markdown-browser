import { LitElement, html, css, type CSSResultGroup } from 'lit';
import { customElement, property, query } from 'lit/decorators.js';
//import '@awesome.me/webawesome/dist/styles/webawesome.css'; // this imports all the styles
import '@awesome.me/webawesome/dist/styles/themes/default.css'; // alternative to importing all css -- but does it shake out?
import '@awesome.me/webawesome/dist/components/drawer/drawer.js';
import '@awesome.me/webawesome/dist/components/button/button.js';
import type WaDrawer from '@awesome.me/webawesome/dist/components/drawer/drawer.js';

/**
 * An example element.
 *
 * @fires count-changed - Indicates when the count changes
 * @slot - This element has a slot
 * @csspart button - The button
 */
@customElement('mbr-info')
export class MbrInfoElement extends LitElement {
  static override styles: CSSResultGroup = css`
    :host {
      display: block;
      border: solid 1px gray;
      padding: 16px;
      max-width: 800px;
    }
  `;

  constructor() {
    super();
    this.name = "hi";
  }

  /**
   * The name to say "Hello" to.
   */
  @property()
  name = 'World';

  /**
   * The number of times the button has been clicked.
   */
  @property({ type: Number })
  count = 0;

  @query("wa-drawer")
  _drawer!: WaDrawer;

  override render() {
    return html`
      <slot></slot>
      <wa-drawer label="Browse">
        <h1>${this.sayHello(this.name)}!</h1>
        <wa-button slot="footer" variant="brand" data-drawer="close">Close</wa-button>
      </wa-drawer>
      <wa-button @click=${this._onClick}>Open Drawer</wa-button>
    `;
  }

  private _onClick() {
    this._drawer.open = true;
  }

  /**
   * Formats a greeting
   * @param name The name to say "Hello" to
   */
  sayHello(name: string): string {
    return `Hello, ${name}`;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-info': MbrInfoElement;
  }
}
