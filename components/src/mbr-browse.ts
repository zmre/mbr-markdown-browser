import { siteNav } from './shared.ts';
import { customElement, state } from 'lit/decorators.js';
import { LitElement, css, html, type TemplateResult } from 'lit'

interface Folder {
  [folder: string]: Folder
}

@customElement('mbr-browse')
export class MbrBrowseElement extends LitElement {
  @state()
  private _markdownFiles: any[] = [];

  @state()
  private _otherFiles: any[] = [];

  @state()
  private _folderHierarchy: Folder = {};

  constructor() {
    super();
    siteNav.then((nav) => {
      console.log(nav);
      // {
      //     mentalism: {
      //       gimmicks: {
      //         fraud-credit-card: {}
      //       }
      //     },
      //     cash: {
      //      tricks: {
      //        extreme-burn: {},
      //        washington: {},
      //     }
      // }
      //
      // alternative:
      // {
      //   mentalism: {
      //      title: null,
      //      path: "/mentalism"
      //      children: {
      //        gimmicks: {
      //          path: "/mentalism/gimmicks"
      //          title: null,
      //          children: {
      //            fraud-credit-card: {
      //              title: "card",
      //              path: "/mentalism/gimmicks/fraud-credit-card"
      //              children: {}
      //            }
      //          }
      //        }
      //      }
      //   }
      // }
      if (nav?.markdown_files) {
        let tree: Folder = {};
        let cur = tree;
        for (const mdentry of nav.markdown_files) {
          for (const folder of (mdentry["url_path"] as string)?.split('/')) {
            if (folder && folder !== ".." && folder !== ".") {
              cur[folder] = cur[folder] ?? {};
              cur = cur[folder];
            }
          }
          cur = tree;
        }
        this._markdownFiles = nav.markdown_files;
        this._folderHierarchy = tree;
        // this._markdownFiles = nav.markdown_files;
      }
      if (nav?.other_files) {
        this._otherFiles = nav.other_files;
      }
      console.log(this._folderHierarchy);
    })
  }

  folderHierarchy2List(folders: Folder, path: string) {
    return Object.keys(folders).sort().reduce((prev: TemplateResult<1>, key: string) => {
      let children = html``;
      if (folders[key]) {
        // if (Object.keys(folders[key]).length > 0) {
        children = html`<ul>${this.folderHierarchy2List(folders[key], path + "/" + key)}</ul>`
      }
      return html`${prev}<li><a href="${path}/${key}">${key}</a>${children}</li>`;
    }, html``);
  }

  static override styles = css`
  `;

  override render() {
    return html`
      Browse; number of markdown files: ${this._markdownFiles.length}; number of other files: ${this._otherFiles.length}

      <ul>
        ${this.folderHierarchy2List(this._folderHierarchy, "")}
      </ul>
    `;
  }

}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-browse': MbrBrowseElement
  }
}
