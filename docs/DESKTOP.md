# Nova Desktop

Nova Desktop is not a themed text console. It is a native, allocation-free
framebuffer shell rendered by the Rust kernel while the process/compositor layer
is being built. Version 0.3 replaces the early sidebar shell with the approved
desktop design: aurora wallpaper, layered application windows, a floating
taskbar, Nova AI and a quick Control Center.

## Current interaction model

- Home composes Files, Nova AI and quick settings as independent surfaces.
- Files renders real RAM filesystem entries as selectable file tiles.
- Terminal is a dedicated maximized window that executes system tools.
- Nova AI has a focused operator window with access and task state.
- Control Center exposes network, sound, brightness and power surfaces.
- The floating taskbar switches applications and shows active state.
- PS/2 mouse packets drive a software XOR cursor and clickable navigation.
- F1-F4 and Escape provide a complete keyboard fallback.

The UI scales its windows and taskbar for the detected framebuffer. Its visual
language uses a generated aurora, dark glass surfaces, cyan system light, yellow
Files, blue tools, purple AI actions, window shadows and compact typography.

## Path to a Windows-class shell

This milestone establishes the interaction and rendering contract. The next
desktop work is a double-buffered compositor, damage tracking, movable/resizable
windows, a task switcher, launcher/search, notifications, settings, an editor,
font shaping and accessibility. Those services will move out of the kernel after
Nova has ring-3 processes and capability IPC.
