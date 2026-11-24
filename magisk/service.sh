#!/system/bin/sh
MODDIR="${0%/*}"

while [ "$(getprop sys.boot_completed)" != "1" ]; do
    sleep 1
done

if [ "$(getprop persist.vendor.sys.fp.vendor)" != "goodix_fod" ]; then
    setprop persist.vendor.sys.fp.vendor goodix_fod
fi

if [ "$(getprop persist.sys.psyche.keep_umi_aod)" != "1" ]; then
    stop umi_aod_bridge 2>/dev/null
    killall umi_aod_bridge 2>/dev/null
fi

if [ -x "$MODDIR/system/bin/psyche_aodd" ]; then
    "$MODDIR/system/bin/psyche_aodd" >/dev/null 2>&1 &
fi
