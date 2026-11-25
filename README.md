# Wiki

Wiki is a Rust voice assistant for 64-bit Windows 10 and 11. It runs in the notification area and listens locally for **“Hey Wiki.”**

## Run Wiki

Build from PowerShell with the Rust MSVC toolchain and Visual Studio Build Tools installed:

```powershell
.\build-wiki.ps1
Start-Process .\target\wiki-app\wiki.exe
```

The first build downloads a pinned Vosk runtime and an English speech model. Keep the model and DLLs beside `wiki.exe` when moving the app. After setup, speech recognition works offline. Web searches and websites use their normal internet connection.

If Wiki reports a microphone error, make sure Windows has a default input device and allows desktop apps to use the microphone. Click **Retry microphone** after changing those settings.

## Use voice commands

Say **“Hey Wiki,”** wait for the animated orb at the bottom of your desktop, then speak one command within eight seconds. The orb also appears when Wiki is in the tray. Once speech recognition finishes, the final transcript appears below the orb for three seconds.

| Say | What Wiki does |
| --- | --- |
| Open Chrome | Launches Google Chrome. |
| Open ChatGPT | Opens ChatGPT in your default browser. |
| Play, Pause, or Play or pause | Sends the Windows play/pause media key. All three phrases toggle the player Windows routes the key to. |
| Switch apps | Sends Alt+Tab to return to the previous window. |
| Search for *something* | Searches Google for the spoken words in Chrome. |

Use **Add command** to bind a phrase to an app executable, website, media toggle, app switch, or Chrome search. **Edit** changes or removes an existing binding. Wiki saves commands in `%LOCALAPPDATA%\Wiki\commands.json`.

Closing the window moves Wiki to the tray. Click the tray icon to reopen it, use **Pause listening** to release the microphone, or choose **Quit Wiki** from the tray menu to exit. To start Wiki at sign-in, place a shortcut to `wiki.exe --tray` in the Windows Startup folder (`shell:startup`).

## Add action types

Voice capture and transcription live in `speech.rs` and `vosk.rs`. `session.rs` controls wake and command timing. `commands.rs` owns action types, phrase matching, and storage, while `actions.rs` dispatches to Windows. To add a new action such as volume or brightness, add its action type, implement it in the dispatcher, and expose it in the editor. The wake-word and microphone pipeline stays shared.

Run the regular test suite with `cargo test --locked`.
