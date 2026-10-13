# Flow

Flow is a desktop manager for AI packages and Civitai models, built with TypeScript, CSS, and Tauri v2.

## Development

Requires Node.js 24, pnpm 11, Rust, Git, and the [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/).

```powershell
pnpm install
pnpm start
```

Build the Windows installer with `pnpm dist:win`. GitHub Actions also builds an NSIS `.exe` on every push.

## Settings

- Use the header control to switch between light and dark themes. The preference is saved in `localStorage`.
- In Settings, choose an existing parent folder with **Обзор...**. Flow saves this path locally and installs AUTOMATIC1111 in its `sd-webui` subfolder. The same folder can be selected in the install dialog.
- A Civitai API key can be saved in Settings if required for restricted models. `CIVITAI_API_KEY` can also be supplied as an environment variable.
