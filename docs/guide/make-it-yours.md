# Make it yours

Back to the [tour](../../README.md). Everything here is in **Customize**; nothing is final until **Save changes**, and **Cancel** brings back what you had.

## Colours

* **Mode:** Dark, Light or System (follows the desktop).
* **Palette:** background, text, accent, muted, warning and error, each with a colour picker.
* **Follow the wallpaper colours.** One checkbox. Status Quo shrinks your wallpaper to 64 pixels, takes its dominant hue and builds a palette for the mode you chose, moving every colour until it reads well on the background (text at least 7:1, the others at least 4.5:1). While it is on, the swatches show the colours actually in use and cannot be edited. A grey wallpaper gives a neutral palette, because a hue that is not there is never invented. If the picture cannot be found or read, your palette stays in use and the reason is printed next to the checkbox.
* **Opacity** (background only, text stays sharp), **font size** (10 to 32), **compact spacing**, and the **space between widgets** (one value for the whole tray).
* **Per widget:** tick *Override widget appearance* on a widget to give it its own look, including its own wallpaper option.

### Which wallpaper does it read?

| Desktop | Source read | 
|---|---|
| Any, when set | the picture named by the environment variable `STATUS_QUO_WALLPAPER` |
| KDE Plasma | `plasma-org.kde.plasma.desktop-appletsrc` (a wallpaper package folder resolves to its smallest picture) |
| GNOME, Budgie, Unity, Pantheon, Cinnamon, MATE | `gsettings` (`picture-uri-dark` when the colour scheme is dark) |
| Xfce | `xfconf-query` |
| LXQt, LXDE | their settings files |
| nitrogen, feh | `bg-saved.cfg`, `~/.fehbg` |
| Hyprland | `hyprpaper.conf` |
| Windows 10 and 11 | the registry value `WallPaper`, then `TranscodedWallpaper` |

Readable formats: PNG, JPEG, BMP and WebP. Not supported: JPEG XL and AVIF pictures, KDE slideshows and solid-colour backgrounds. It looks again every 30 seconds, but only while some theme follows the wallpaper, and decodes the picture only when its file changed. Verified on KDE Plasma; the others are tested with fixtures, not on real desktops, and Windows is tested with fixtures on Linux.

## Layout

* **Edit tray** (the button on the tray): drag a widget to move it, drag an edge or corner to resize on both axes, press **Remove** in the middle to delete. Borders and sizes snap within 10 pixels to neighbours; a pink line shows an aligned border, a dashed blue line an aligned centre, an amber bar an equal size. Widgets can go anywhere on the desktop. **Done** saves, **Cancel** discards.
* **Lock widget layout** in Customize freezes it; Edit tray is disabled while locked.
* **Order, hide, detach:** in **Customize → Widgets**, each widget can be moved up or down, hidden, or detached into its own window.

## Where the tray sits

| Setting | Where | Meaning |
|---|---|---|
| Window layer | Customize → Desktop behavior | Behind (default), Normal or Above other windows |
| Start at login | same | on by default |
| `STATUS_QUO_HOST` | environment | force `layer-shell`, `x11` or `window` when it can work |
| `STATUS_QUO_ANCHOR` | environment | corner or edge for the layer-shell host: `top`, `bottom`, `left`, `right` or a pair like `bottom-left` (default `top-right`) |

Customization and screenshots always use an ordinary window, because the desktop-layer host has no keyboard input yet.

## What to read

Customize → **Sources** lists the Claude folder, Codex folder, workflow-memory folder and an optional extra progress folder, each with a **Browse…** button (a small folder navigator: Home, Up, drive or root buttons, click a folder to open it, **Use this folder** to keep it). **Local project roots** are the folders where your projects live; **Add a project folder…** works the same way. Progress counts do not need the optional folder: each session's own task list is used.

## Presets and sharing

*Save preset* stores the current look under a name. *Export settings to text* gives your settings as text; *Import into draft* reads text from someone else. Imports are checked before anything changes: a broken or hostile file leaves your working setup untouched, and navigation can be switched off independently so a shared setup cannot open links for you.

## Isolated trials

`STATUS_QUO_STATE_DIR=/some/folder status-quo` runs the whole program with its own settings and history, so you can experiment without touching your real tray.
