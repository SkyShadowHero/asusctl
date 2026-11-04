#!/usr/bin/env bash
# Apply the kernel backlight argument requested by asusd for the current GPU MUX
# mode.
#
# On laptops with a MUX switch the panel is driven by a different GPU depending
# on the mode, and so is the component that actually controls its backlight:
#
#   Ultimate (discrete) : panel on the dGPU -> only nvidia_0 works, which the
#                         NVIDIA driver only installs with acpi_backlight=native
#   Hybrid / Integrated : panel on the iGPU -> nvidia_wmi_ec_backlight works,
#                         and the parameter must NOT be set
#
# asusd is sandboxed (ProtectSystem=strict) so it cannot regenerate the boot
# loader configuration itself; it writes the request to /etc/asusd/gpu_bootargs
# and starts asusd-bootargs.service, which runs this script unsandboxed.

set -euo pipefail

CONF=/etc/default/limine
MARKER=/etc/asusd/gpu_bootargs
CONF_FILE=/boot/limine.conf
ARG=acpi_backlight=native
MANAGED="KERNEL_CMDLINE[default]+=$ARG"

log() { printf 'boot-args: %s\n' "$*"; }

[[ -r $MARKER ]] || { log "no request ($MARKER missing)"; exit 0; }
[[ -f $CONF ]] || { log "$CONF missing, not a limine-entry-tool setup - nothing to do"; exit 0; }

request=$(tr -d '[:space:]' <"$MARKER")
case "$request" in
	native | default) ;;
	*)
		log "unknown request '$request', ignoring"
		exit 0
		;;
esac

# Rebuild the KERNEL_CMDLINE[default] directives: drop every acpi_backlight=
# token (any value, ours or a hand-made one) and append the managed line when
# the parameter is wanted.
rewrite() {
	local line trimmed
	local -a out=()
	while IFS= read -r line || [[ -n $line ]]; do
		trimmed=${line#"${line%%[![:space:]]*}"}
		[[ $trimmed == "$MANAGED" ]] && continue
		if [[ $trimmed == KERNEL_CMDLINE\[* ]]; then
			line=$(printf '%s' "$line" | sed -E \
				-e 's/"acpi_backlight=[[:alnum:]_]+//' \
				-e 's/[[:space:]]+acpi_backlight=[[:alnum:]_]+//g')
		fi
		out+=("$line")
	done <"$CONF"
	if [[ $request == native ]]; then
		out+=("$MANAGED")
	fi
	printf '%s\n' "${out[@]}"
}

old=$(cat "$CONF")
new=$(rewrite)
if [[ $new == "$old" ]]; then
	log "already correct ($request)"
	exit 0
fi

backup="$CONF.bak-asusd"
cp -a "$CONF" "$backup"
printf '%s\n' "$new" >"$CONF"
log "writing $CONF ($request)"

if ! limine-update; then
	log "limine-update failed, restoring $CONF"
	cp -a "$backup" "$CONF"
	exit 1
fi

# Verify the generated entries actually carry (or dropped) the parameter.
if [[ -r $CONF_FILE ]]; then
	if [[ $request == native ]] && ! grep -q -- "$ARG" "$CONF_FILE"; then
		log "$CONF_FILE does not contain $ARG, restoring"
		cp -a "$backup" "$CONF"
		limine-update || true
		exit 1
	fi
	if [[ $request != native ]] && grep -q -- 'acpi_backlight=' "$CONF_FILE"; then
		log "$CONF_FILE still contains acpi_backlight=, restoring"
		cp -a "$backup" "$CONF"
		limine-update || true
		exit 1
	fi
fi

log "done ($request), effective after reboot"
