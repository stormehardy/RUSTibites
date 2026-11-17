# Native 1920 runtime UI dumps + UI-on-black menu refs

`ui_runtime_native.json.gz` holds native `/d/ui` node trees for `native_menu_title`,
`native_menu_settings`, and `native_world_ui`; screen is 1920x1080. UI-on-black menu images are captured by
hiding BackGround/Title Screen after activating/normalizing the Welcome UI; settings image is crisp 1920
(panel at native coordinates), title image is sourced from the verified `probe_title_bg_off` native capture.
The title panel image bbox is ~[1366,64,1877,900]; the old `ui_runtime.json.gz` was captured at 320x240 and is
historical only. This is the first-pass title/settings/world verification before bulk node recapture.
