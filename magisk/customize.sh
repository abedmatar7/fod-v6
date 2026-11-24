#!/system/bin/sh
ui_print "- Installing NEESS ColorOS 17 FOD & AOD Fix v6.0 All-in-One (Xiaomi 12X / psyche)"
set_perm_recursive "$MODPATH" 0 0 0755 0644
[ -f "$MODPATH/system/bin/psyche_aodd" ] && set_perm "$MODPATH/system/bin/psyche_aodd" 0 2000 0755
[ -f "$MODPATH/system/vendor/bin/psyche_extcmd" ] && set_perm "$MODPATH/system/vendor/bin/psyche_extcmd" 0 2000 0755 u:object_r:vendor_file:s0
[ -f "$MODPATH/system/vendor/lib64/vendor.xiaomi.hardware.fx.tunnel@1.0.so" ] && set_perm "$MODPATH/system/vendor/lib64/vendor.xiaomi.hardware.fx.tunnel@1.0.so" 0 0 0644 u:object_r:same_process_hal_file:s0
set_perm "$MODPATH/post-fs-data.sh" 0 0 0755
set_perm "$MODPATH/service.sh" 0 0 0755
