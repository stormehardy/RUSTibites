# Native 1920 menu background-only

Captured standalone hidden at native 1920. UI hidden by deactivating `UI`, `Title Screen`, and
`UI/Popuholder/screenBlocker`, keeping `BackGround` active. Frozen with `Time.timeScale=0` and
`VideoPlayer.Pause()`. Title and Settings use the same background frame (identical hashes).

- `menu_background_title_1920x1080.png` (also title_canvas, settings) — blurred live-world/video background.
- Video info from `/d/menu_background_info`: VideoClip **showOff**, RenderTexture **Video 1920×1080**;
  player target texture Video; frozen during capture. The background is a VideoClip played into a
  RenderTexture by `BackGround/Video Player/VideoTexture`; it is animated in normal play but frozen here.
- Use this frame to composite behind native UI; settings and other menu screens share it.
