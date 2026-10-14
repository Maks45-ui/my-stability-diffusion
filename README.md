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

- Open theme settings from the header. Choose Light, Dark, System, Заказ, Багровая Луна, or Синелетовые сумерки. The choice is saved in `localStorage`; System follows Windows color scheme changes while Flow is open.
- In Settings, choose an existing parent folder with **Обзор...**. Flow saves this path locally and installs AUTOMATIC1111 in its `sd-webui` subfolder. The same folder can be selected in the install dialog.
- A Civitai API key can be saved in Settings if required for restricted models. `CIVITAI_API_KEY` can also be supplied as an environment variable.

## Model downloads

The Models page keeps a download queue with progress, transfer speed, and estimated time remaining. Pause preserves a partial file so **Continue** can resume it, including after restarting Flow. **Cancel** stops an active transfer and removes its partial file. **Delete** removes a completed or paused model from Flow's model directory.

The welcome animation covers the app window on the first view of each session. It fades into the main screen and respects the system reduced-motion setting.

## Interface material

Panels, cards, the sidebar, notifications, and dialogs share the `glass-surface` style in `src/glass.css`. Each palette in `src/themes.css` supplies its own translucent tint, edge highlight, and layered shadows. Hover strengthens the sheen and shadow without changing the sidebar width or increasing blur every frame. Reduced-transparency and unsupported-backdrop fallbacks use solid surfaces.
