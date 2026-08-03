# tetron guide

How to use tetron and its clients: the Android app, this browser dashboard, and the menu-bar tray. If you landed here from a release page and have nothing installed yet, start with whichever client matches your device.

## What is tetron

tetron is a peer-to-peer mesh VPN daemon — devices join a shared private network directly, without routing through a central server. The daemon itself is CLI-only by design ([do one thing well](https://github.com/ErikAllanKincaid/tetron)); everything below is a separate, optional client that talks to it. For the full CLI reference, network/admin concepts in depth, and installing the daemon itself, see [tetron's own HOWTO](https://erikallankincaid.github.io/tetron/HOWTO.html).

Three ideas come up in every client below:

<a id="connection-states"></a>
#### Active / Standby / Suspend

A joined network has three states. **Active** routes your real traffic through the mesh. **Standby** stays connected to other peers (they can still see you're online) but does not route anything. **Suspend** disconnects fully — you become unreachable until you resume.

<a id="connection-types"></a>
#### Direct / Relay / Tor

How you're currently reaching a given peer. **Direct** is a straight peer-to-peer path. **Relay** goes through a relay server because a direct path isn't available (still end-to-end encrypted). **Tor** routes through the Tor network.

<a id="roles"></a>
#### Admin / member

Every network has one or more admins (can invite/remove people) and members (everyone else). Shown as a small role marker next to each peer.

## tetron-mobile (Android)

The Android client. Proprietary, separate from tetron's own open-source code, but it embeds the same daemon directly on-device rather than talking to it over a socket — installing the app is enough, there's nothing else to set up.

**Install:** download the APK from [tetron's GitHub releases](https://github.com/ErikAllanKincaid/tetron/releases) (look for the latest `tetron-mobile-android-*.apk` asset — it's mirrored there from the app's own release).

**Join a network:** open the app, tap **Manage**, and either scan a QR invite code or paste one in manually. Give the device a name first (prefilled from your device model) — that's how you'll show up to other members.

**Active / Standby:** the switch on the Home screen. A freshly-joined network starts in Standby, not Active — flip it on to actually route traffic.

**Suspend / Resume:** the "Suspend" link near the Active/Standby switch fully disconnects you; **Resume** on the same screen brings you back without needing a new invite.

**Peers:** Home lists everyone on the network, with their IP and connection type (Direct/Relay/Tor). Tap any row to copy its IP, or tap the network name to copy it.

**Leave:** open **Manage** on a joined network and use **Leave network**. This can't be undone from the app — rejoining needs a fresh invite.

## tetron-webui

A browser dashboard for the daemon — this page's own repo. Create or join networks, manage invites and admins, and install/uninstall the other add-ons (including this one) from its Add-ons panel. Runs locally alongside the daemon; open it at whatever address it's configured to listen on. See [`tetron-webui`](https://github.com/ErikAllanKincaid/tetron-webui) for setup.

## tetron-systray

A menu-bar/tray client — glanceable status and a quick per-network Active/Standby toggle, without opening a browser. Same daemon, same IPC socket, install alongside or instead of the webui. See [`tetron-systray`](https://github.com/ErikAllanKincaid/tetron-systray) for setup.
