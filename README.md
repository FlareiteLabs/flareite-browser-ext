# Browser

Web links you Ctrl+click (Cmd+click on macOS) open in a tab that shows the page, instead of in
your browser: in the terminal's output, in the editor's text, and in previews.

- The tab's header shows where the page is, with **Back**, **Forward**, **Reload**, **Go to an
  Address…**, and **Open in Your Browser**. The tab takes the page's title.
- **Browser: Open Address…** in the command palette opens a tab at an address you type
  (`example.com` is enough).
- **Ctrl+Shift+click** a link to choose this time between your browser and a tab. To keep using
  your browser by default, set `workbench.open_web_links = "default"` in Flareite's settings.
- Sign in to sites and fill in forms as in a browser. Pages can't reach Flareite or your files.

## Permissions

- **Show pages from the web in its tabs.** Flareite asks before the extension shows one.

## Building

The extension is written in Rust against Flareite's extension API, in the editor's repository,
which it expects beside this one as `editor`:

```sh
cargo build --release
```

Then load this folder with **Extensions: Load Development Extension…**. See Flareite's
`docs/extensions.md` (Links, and Layouts) for the API it uses: a link handler
(`[[link_handlers]]`, answered by `open-link`) and a layout with a web page pane
(`layout-fill` and `pane-navigated` events, `layouts::navigate-pane`).
