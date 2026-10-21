#!/usr/bin/env bash
# Keep test windows (game client, launcher, agent browser) on their own KWin
# virtual desktop so unattended tests don't interrupt work on the other desktops.
#
#   scripts/agent-desktop.sh on      create the "Agents" desktop (once) and force the rules
#   scripts/agent-desktop.sh off     remove the rules (the desktop stays)
#   scripts/agent-desktop.sh status
#
# The game rule matches every Proton window of app 2694490, including the
# official game started from Steam. Turn the rules off before playing or
# recording an official capture.
#
# Web UI checks in a visible browser use a separate Firefox instance:
#   firefox --new-instance --name poe-agent-browser --class poe-agent-browser \
#           --profile runtime/agent-browser-profile <url>
set -euo pipefail

DESKTOP_NAME="Agents"
RULES_FILE="$HOME/.config/kwinrulesrc"
DBUS=(qdbus6 org.kde.KWin /VirtualDesktopManager)

# id<TAB>name<TAB>wmclass regex
RULES=(
  "poe-agent-game	PoE2 client -> Agents desktop	steam_app_2694490|pathofexilesteam\\.exe"
  "poe-agent-launcher	Poe2 Launcher -> Agents desktop	poe2-launcher|poe2_launcher|com\\.distorbed\\.poe2launcher"
  "poe-agent-browser	Agent browser -> Agents desktop	poe-agent-browser"
)

desktop_id() {
  qdbus6 --literal org.kde.KWin /VirtualDesktopManager org.kde.KWin.VirtualDesktopManager.desktops \
    | grep -oP '\(uss\) \d+, "\K[0-9a-f-]+(?=", "'"$DESKTOP_NAME"'")' || true
}

ensure_desktop() {
  local id
  id=$(desktop_id)
  if [[ -z "$id" ]]; then
    local count
    count=$("${DBUS[@]}" org.kde.KWin.VirtualDesktopManager.count)
    "${DBUS[@]}" org.kde.KWin.VirtualDesktopManager.createDesktop "$count" "$DESKTOP_NAME" >/dev/null
    id=$(desktop_id)
  fi
  [[ -n "$id" ]] || { echo "could not create the $DESKTOP_NAME desktop" >&2; exit 1; }
  echo "$id"
}

# Plasma 6 keeps the rule order in [General] Order (older: rules + count; KWin migrates them).
rule_list() {
  local order
  order=$(kreadconfig6 --file "$RULES_FILE" --group General --key Order)
  [[ -n "$order" ]] && echo "$order" || kreadconfig6 --file "$RULES_FILE" --group General --key rules
}

set_rule_list() {
  kwriteconfig6 --file "$RULES_FILE" --group General --key Order "$1"
  kwriteconfig6 --file "$RULES_FILE" --group General --key rules --delete
  kwriteconfig6 --file "$RULES_FILE" --group General --key count --delete
}

without_ours() {
  tr ',' '\n' <<<"$(rule_list)" | grep -v '^poe-agent-' | grep . | paste -sd, - || true
}

reconfigure() { qdbus6 org.kde.KWin /KWin reconfigure >/dev/null; }

case "${1:-status}" in
  on)
    id=$(ensure_desktop)
    list=$(without_ours)
    for r in "${RULES[@]}"; do
      IFS=$'\t' read -r key desc class <<<"$r"
      w=(kwriteconfig6 --file "$RULES_FILE" --group "$key")
      "${w[@]}" --key Description "$desc"
      "${w[@]}" --key wmclass "$class"
      "${w[@]}" --key wmclassmatch 3        # regex
      "${w[@]}" --key wmclasscomplete false
      "${w[@]}" --key desktops "$id"
      "${w[@]}" --key desktopsrule 2        # force
      "${w[@]}" --key fsplevel 4            # focus stealing prevention: extreme
      "${w[@]}" --key fsplevelrule 2
      "${w[@]}" --key fpplevel 4            # focus protection: extreme
      "${w[@]}" --key fpplevelrule 2
      list="${list:+$list,}$key"
    done
    set_rule_list "$list"
    reconfigure
    echo "rules on: game, launcher and agent browser go to desktop '$DESKTOP_NAME' ($id)"
    ;;
  off)
    for r in "${RULES[@]}"; do
      IFS=$'\t' read -r key _ _ <<<"$r"
      kwriteconfig6 --file "$RULES_FILE" --group "$key" --key Description --delete
      sed -i "/^\[$key\]$/,/^\[/{/^\[$key\]$/d;/^\[/!d}" "$RULES_FILE"
    done
    set_rule_list "$(without_ours)"
    reconfigure
    echo "rules off"
    ;;
  status)
    echo "desktop '$DESKTOP_NAME': $(desktop_id || true)"
    echo "rules: $(rule_list)"
    ;;
  *) echo "usage: $0 on|off|status" >&2; exit 2 ;;
esac
