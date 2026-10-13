# Flow

Настольный менеджер пакетов и моделей на строгом TypeScript, Electron и CSS.

## Запуск

Требуется Node.js 24 и pnpm 11.25.0.

```powershell
pnpm install
pnpm start
```

Для Windows-сборки `.exe` с необходимыми файлами:

```powershell
pnpm run dist:win
```

Пакеты и модели сохраняются в пользовательской папке Flow, а не внутри установленного приложения. Для AUTOMATIC1111 WebUI нужны Git и Python 3.10. Ключ Civitai можно указать в разделе «Настройки» или через переменную окружения `CIVITAI_API_KEY`.

При отправке изменений в GitHub Actions создаётся артефакт `Flow-Windows` с `Flow.exe` и необходимыми файлами рядом с ним.

