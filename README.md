# Wiki

[فارسی](README-fa.md)

<img src="assets/Wiki%20the%20boy.png" width="96" alt="Wiki mascot">

Say **Hey Wiki** to open a favorite site, search Google in Chrome, or switch apps with your voice. Wiki is a voice assistant for 64-bit Windows 10 and 11. It hangs out in the system tray and recognizes English speech on your computer.

## Download Wiki

For a published version, download `wiki-windows-x64.zip` from GitHub Releases, extract it, and open `wiki.exe` in the extracted `wiki-app` folder. Keep the files in that folder together.

## Build and run

You'll need the 64-bit Rust MSVC toolchain and Visual Studio Build Tools with the Windows SDK. Open PowerShell in the project folder and run both commands.

```powershell
.\build-wiki.ps1
Start-Process .\target\wiki-app\wiki.exe
```

The first build downloads Vosk and its English speech model. Speech recognition works offline after that. Websites and searches still use your internet connection. If you move Wiki, move the whole `target/wiki-app` folder so the model and DLLs stay next to `wiki.exe`.

If Wiki can't open the microphone, check that Windows has a default input device and that desktop apps have microphone access. Then click **Retry microphone** in Wiki.

## Try a command

Say **Hey Wiki** and wait for the animated orb at the bottom of your desktop. You have eight seconds to say one command. The orb appears even when Wiki is in the tray, and the final transcript stays below it for three seconds.

| Say | What Wiki does |
| --- | --- |
| `Open Chrome` | Opens Google Chrome. |
| `Open ChatGPT` | Opens ChatGPT in your default browser. |
| `Play`, `Pause`, or `Play or pause` | Sends the same Windows play/pause key to toggle playback. |
| `Switch apps` | Presses Alt+Tab to switch to the previous window. |
| `Search for easy pasta recipes` | Searches Google in Chrome for “easy pasta recipes.” Replace those words with whatever you want to find. |

Want Wiki to open a site you use every day? Click **Add command**, pick an English phrase, and choose an action such as opening a site or launching an app. Use **Edit** to change or delete a command. Wiki saves your commands in `%LOCALAPPDATA%\Wiki\commands.json`.

## Keep Wiki in the tray

Close the main window and Wiki keeps listening from the tray. Click the tray icon to reopen it. Use **Pause listening** to release the microphone, or choose **Quit Wiki** from the tray menu to exit.

To start Wiki at sign-in, open `shell:startup` from the Windows Run dialog. Put a shortcut to `wiki.exe` in that folder and add `--tray` to the end of its Target field.

## Work on Wiki

Wiki uses Rust, egui, CPAL, and Vosk. If you want to add an action, define it in `src/commands.rs`, implement it in `src/actions.rs`, and add it to the command editor in `src/ui.rs`.

Run the regular test suite from the project folder.

```powershell
cargo test --locked
```

## Publish a Windows release

Push a tag like `v0.1.0` on a commit that includes the release workflow. GitHub Actions runs the tests, builds Wiki, and publishes the Windows ZIP as a release asset.

```powershell
git tag v0.1.0
git push origin v0.1.0
```
