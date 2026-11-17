using System;
using System.Collections;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Net;
using System.Reflection;
using System.Text;
using System.Threading;
using BepInEx;
using HarmonyLib;
using BepInEx.Configuration;
using BepInEx.Logging;
using UnityEngine;

namespace RUSTibitesParityDump
{
    /// <summary>
    /// RUSTibites parity dump plugin (unique GUID; independent of the BibitesBridge plugin).
    ///
    /// Why it exists: the ground-truth parity harness needs two things that the existing BibitesAPI bridge
    /// cannot provide:
    ///   1. prefab / serialized constants that live only in the game's scene assets (matter material densities,
    ///      bibite prefab Rigidbody2D / CapsuleCollider2D settings, Physics2D configuration and the collision
    ///      layer matrix, ColorKiller/RadTower/PheroTower prefab defaults) - they are absent from save files and
    ///      only *declared* (not valued) in the decompiled C#;
    ///   2. a frame-rate-independent tick: the game's FPS governor (TimeController.CheckMinFPS) rescales
    ///      Time.fixedDeltaTime while it ramps engineTimeScale back to 1, so the dt of the first few ticks after
    ///      a world load depends on real frame timing. POST /d/pacing {"lock":true} pins Time.fixedDeltaTime (and
    ///      Time.maximumDeltaTime) to TimeController.baseFixedDeltaTime = 1/simTPS every frame, which is the
    ///      steady-state 1x behaviour the game runs at when it is not throttled, and what RUSTibites targets.
    ///
    /// Everything game-side is reached by REFLECTION (type names are looked up in the loaded assemblies), so this
    /// plugin does not compile against - and cannot be broken by - a specific BibitesAssembly build.
    ///
    /// Routes (HTTP on 127.0.0.1:17801, override with env BIBITES_DUMP_PORT or BepInEx config):
    ///   GET  /d/ping                     -> {"ok":true,"plugin":...,"unity":...}
    ///   GET  /d/dump                     -> the full constants JSON (main thread)
    ///   GET  /d/pacing                   -> {"lock":bool,"fixedDeltaTime":..,"maximumDeltaTime":..,"timeScale":..}
    ///   POST /d/pacing {"lock":true}     -> enable/disable the dt pin
    ///   GET  /d/time                     -> time/engine snapshot (paused, simulatedTime via reflection)
    /// </summary>
    [BepInPlugin(Guid, "RUSTibites Parity Dump", Version)]
    [DefaultExecutionOrder(20000)]              // after the game's own scripts in a frame, before the next FixedUpdate batch
    public class Plugin : BaseUnityPlugin
    {
        public const string Guid = "com.rustibites.paritydump";
        public const string Version = "0.1.0";

        internal static ManualLogSource Log;
        private static Harmony _feedingHarmony;
        private static bool _harmonyTried;
        private ConfigEntry<int> _portCfg;
        private ConfigEntry<bool> _enabled;
        private HttpListener _listener;
        private Thread _thread;
        private volatile bool _running;
        private int _port;

        // ---- main-thread marshalling -----------------------------------------------------------------
        private static readonly ConcurrentQueue<Action> Queue = new ConcurrentQueue<Action>();

        private static T OnMain<T>(Func<T> fn, int timeoutMs = 60000)
        {
            T result = default(T);
            Exception error = null;
            var done = new ManualResetEventSlim(false);
            Queue.Enqueue(() =>
            {
                try { result = fn(); }
                catch (Exception e) { error = e; }
                finally { done.Set(); }
            });
            if (!done.Wait(timeoutMs)) throw new TimeoutException("the game's main thread did not run the request in time");
            if (error != null) throw new Exception(error.GetType().Name + ": " + error.Message, error);
            return result;
        }

        // ---- dt pin ----------------------------------------------------------------------------------
        private volatile bool _dtPin;
        private static FieldInfo _baseDtField;
        private static Type _timeControllerType;

        private void SetDtPin(bool on)
        {
            _dtPin = on;
            Log.LogInfo("parity dt pin " + (on ? "ON" : "OFF") + " (fixedDeltaTime=" + ReadBaseFixedDeltaTime().ToString("R", CultureInfo.InvariantCulture) + ")");
        }

        /// <summary>TimeController.baseFixedDeltaTime = 1/simTPS, maintained by the game whenever simTPS changes.
        /// Read it every frame while pinning: the value is a placeholder (0.025) until TimeController.Awake has run
        /// in the simulation scene, so caching it once can pin the wrong dt.</summary>
        internal static float ReadBaseFixedDeltaTime()
        {
            try
            {
                if (_timeControllerType == null) _timeControllerType = FindType("ManagementScripts.TimeController");
                if (_timeControllerType != null)
                {
                    if (_baseDtField == null) _baseDtField = _timeControllerType.GetField("baseFixedDeltaTime", BindingFlags.Static | BindingFlags.NonPublic);
                    if (_baseDtField != null)
                    {
                        float dt = (float)_baseDtField.GetValue(null);
                        if (dt > 0f) return dt;
                    }
                }
            }
            catch { /* fall through */ }
            // fallback: 1/simTPS from the settings, then Unity's default
            try
            {
                var ind = FindType("SettingScripts.ScenarioIndependentSettings");
                var inst = ind.GetField("Instance", BindingFlags.Static | BindingFlags.Public).GetValue(null);
                var tps = ind.GetField("simTPS", BindingFlags.Instance | BindingFlags.Public).GetValue(inst);
                var val = tps.GetType().GetProperty("val", InstanceFlags).GetValue(tps);
                return 1f / Convert.ToInt32(val);
            }
            catch { /* fall through */ }
            return Time.fixedDeltaTime;
        }

        private void PinTickDt()
        {
            if (!_dtPin) return;
            try
            {
                float dt = ReadBaseFixedDeltaTime();
                if (dt <= 0f) return;
                if (Math.Abs(Time.fixedDeltaTime - dt) > 1e-9f) Time.fixedDeltaTime = dt;
                if (Math.Abs(Time.maximumDeltaTime - dt) > 1e-9f) Time.maximumDeltaTime = dt;
            }
            catch { /* never let the pin throw inside Update */ }
        }

        // ---- pause-at-load hook ---------------------------------------------------------------------
        // The game always resumes playing when a save finishes loading; a capture that wants to start
        // from EXACTLY the saved state (no simulation ticks, no RNG draws before /lab/seed) needs the
        // pause to happen inside the game's own load completion callback, before the first FixedUpdate.
        private volatile bool _pauseOnLoad;
        private object _hookedSaveSystem;

        private void SetLoadPause(bool on)
        {
            _pauseOnLoad = on;
            Log.LogInfo("parity pause-on-load " + (on ? "ARMED" : "disarmed"));
        }

        private JObj LoadPauseSnapshot()
        {
            var o = new JObj();
            o.Add("pauseOnLoad", _pauseOnLoad);
            o.Add("saveSystemHooked", _hookedSaveSystem != null);
            o.Add("hookedInstance", _hookedSaveSystem != null ? _hookedSaveSystem.GetHashCode() : 0);
            try
            {
                var tc = FindType("ManagementScripts.TimeController");
                var pf = tc != null ? tc.GetField("paused", StaticFlags) : null;
                o.Add("paused", pf != null ? Convert.ToBoolean(pf.GetValue(null), CultureInfo.InvariantCulture) : false);
            }
            catch { }
            return o;
        }

        private void MaintainLoadHook()
        {
            try
            {
                var t = FindType("ManagementScripts.SaveSystem");
                if (t == null) return;
                var inst = Static(t, "instance");
                if (inst == null) { _hookedSaveSystem = null; return; }
                if (ReferenceEquals(inst, _hookedSaveSystem)) return;
                AttachLoadHook(inst, t);
            }
            catch (Exception e) { Log.LogWarning("parity load hook: " + e.Message); }
        }

        private void AttachLoadHook(object inst, Type saveSystemType)
        {
            var evField = saveSystemType.GetField("onLoadingDone", InstanceFlags);
            var ev = evField != null ? evField.GetValue(inst) as UnityEngine.Events.UnityEvent : null;
            if (ev == null) return;
            ev.AddListener(new UnityEngine.Events.UnityAction(OnLoadingDone));
            _hookedSaveSystem = inst;
            Log.LogInfo("parity load-pause hook attached to SaveSystem instance " + inst.GetHashCode());
        }

        // ---- preload settings + scene hook ------------------------------------------------------------
        // SceneManager.sceneLoaded fires AFTER Awake and BEFORE Start, i.e. before SimulationManager loads
        // the save. Applying the parity settings there means the world loads with the pinned simTPS and
        // with random spawners (virginSpawnRate) already at 0, so a load is a pure restore of the save.
        private readonly Dictionary<string, object> _preload = new Dictionary<string, object>(StringComparer.Ordinal);

        private void OnSceneLoaded(UnityEngine.SceneManagement.Scene scene, UnityEngine.SceneManagement.LoadSceneMode mode)
        {
            try
            {
                ApplyPreloadSettings();
                var t = FindType("ManagementScripts.SaveSystem");
                var inst = t != null ? Static(t, "instance") : null;
                if (inst != null && t != null) AttachLoadHook(inst, t);
            }
            catch (Exception e) { Log.LogWarning("parity sceneLoaded: " + e.Message); }
        }

        private void ApplyPreloadSettings()
        {
            if (_preload.Count == 0) return;
            try
            {
                var t = FindType("SettingScripts.ScenarioIndependentSettings");
                var inst = t != null ? Static(t, "Instance") : null;
                if (inst == null) { Log.LogInfo("parity preload: no ScenarioIndependentSettings yet"); return; }
                int applied = 0;
                foreach (var kv in _preload)
                {
                    var f = t.GetField(kv.Key, InstanceFlags);
                    if (f == null) continue;
                    var setting = f.GetValue(inst);
                    var valProp = setting != null ? setting.GetType().GetProperty("val", InstanceFlags) : null;
                    if (valProp == null || !valProp.CanWrite) continue;
                    object converted = kv.Value;
                    try
                    {
                        if (valProp.PropertyType == typeof(float)) converted = Convert.ToSingle(kv.Value, CultureInfo.InvariantCulture);
                        else if (valProp.PropertyType == typeof(int)) converted = Convert.ToInt32(kv.Value, CultureInfo.InvariantCulture);
                        else if (valProp.PropertyType == typeof(bool)) converted = Convert.ToBoolean(kv.Value, CultureInfo.InvariantCulture);
                    }
                    catch { continue; }
                    valProp.SetValue(setting, converted, null);
                    applied++;
                }
                Log.LogInfo("parity preload: applied " + applied + "/" + _preload.Count + " scenario-independent settings before world load");
            }
            catch (Exception e) { Log.LogWarning("parity preload: " + e.Message); }
        }

        private JObj PreloadSnapshot()
        {
            var o = new JObj();
            var values = new JObj();
            foreach (var kv in _preload.OrderBy(k => k.Key, StringComparer.Ordinal)) values.Add(kv.Key, kv.Value);
            o.Add("values", values);
            return o;
        }

        private void OnLoadingDone()
        {
            if (!_pauseOnLoad) return;
            try
            {
                var tc = FindType("ManagementScripts.TimeController");
                bool paused = false;
                var pf = tc != null ? tc.GetField("paused", StaticFlags) : null;
                if (pf != null) paused = Convert.ToBoolean(pf.GetValue(null), CultureInfo.InvariantCulture);
                if (!paused && tc != null)
                {
                    var inst = Static(tc, "Instance");
                    var mi = tc.GetMethod("TogglePauseGame", InstanceFlags);
                    if (inst != null && mi != null)
                    {
                        mi.Invoke(inst, new object[] { "parity", false });
                        if (pf != null) paused = Convert.ToBoolean(pf.GetValue(null), CultureInfo.InvariantCulture);
                    }
                }
                if (!paused && tc != null)
                {
                    // last-resort freeze: drive engineTimeScale to 0 directly
                    var ets = Static(tc, "engineTimeScale");
                    var valProp = ets != null ? ets.GetType().GetProperty("val", InstanceFlags) : null;
                    if (valProp != null && valProp.CanWrite) valProp.SetValue(ets, 0f, null);
                    Time.timeScale = 0f;
                }
                Log.LogInfo("parity: paused at load completion (paused=" + paused + ", t=" + GetSimulatedTime().ToString("R", CultureInfo.InvariantCulture) + ")");
            }
            catch (Exception e) { Log.LogWarning("parity pause-on-load failed: " + e.Message); }
        }

        internal static double GetSimulatedTime()
        {
            try
            {
                var tk = FindType("OneUseScripts.TimeKeeper");
                var st = tk != null ? tk.GetField("simulatedTime", StaticFlags) : null;
                return st != null ? Convert.ToDouble(st.GetValue(null), CultureInfo.InvariantCulture) : 0.0;
            }
            catch { return 0.0; }
        }

        // ---- feeding census cumulative counters (task-94 follow-up) ---------------------------------
        private readonly Dictionary<int, double> _pelletEnergyCensus = new Dictionary<int, double>();
        private readonly Dictionary<int, int> _pelletZoneCensus = new Dictionary<int, int>();
        private double _censusInitialEnergy;
        private double _censusProducedEnergy;
        private double _censusRemovedEnergy;
        private readonly List<double> _zoneProducedEnergy = new List<double>();
        private readonly List<double> _zoneRemovedEnergy = new List<double>();
        private bool _censusInitialised;

        /// <summary>Install Harmony hooks for exact pellet-energy terms (task-94 completion). Zone.SpawnAPellet's
        /// returned energy = produced; MatterPellet.RemoveAmount is classified as recycled (inside
        /// Zone.RecyclePellets), decayed (inside MatterDecayProcessor.FixedUpdate) or consumed (mouths etc.).
        /// Culled energy is currently 0 (destroy paths are all amount-based); documented as a residual.</summary>
        private static void TryInstallFeedingHarmony()
        {
            if (_harmonyTried) return;
            try
            {
                var zoneType = FindType("SimulationScripts.Zone");
                var pelletType = FindType("SimulationScripts.MatterPellet");
                var decayType = FindType("SimulationScripts.MatterDecayProcessor");
                if (zoneType == null || pelletType == null || decayType == null) return; // retry next frame
                _harmonyTried = true;
                _feedingHarmony = new Harmony("com.rustibites.paritydump.feedingexact");
                var spawn = AccessTools.Method(zoneType, "SpawnAPellet", Type.EmptyTypes);
                var recycle = AccessTools.Method(zoneType, "RecyclePellets", new[] { typeof(float) });
                var decayUpdate = AccessTools.Method(decayType, "FixedUpdate", Type.EmptyTypes);
                var mouthType = FindType("SimulationScripts.BibiteScripts.BibiteMouth");
                var maybeAttack = mouthType == null ? null
                    : AccessTools.Method(mouthType, "MaybeAttack", new[] { typeof(GameObject), typeof(bool) });
                var mouthEnter = mouthType == null ? null
                    : AccessTools.Method(mouthType, "OnTriggerEnter2D", new[] { typeof(Collider2D) });
                var mouthExit = mouthType == null ? null
                    : AccessTools.Method(mouthType, "OnTriggerExit2D", new[] { typeof(Collider2D) });
                var mouthUpdate = mouthType == null
                    ? null : AccessTools.Method(mouthType, "UpdateOrgan", Type.EmptyTypes);
                var t = typeof(FeedingPatches);
                if (spawn != null) _feedingHarmony.Patch(spawn, postfix: new HarmonyMethod(t.GetMethod("SpawnPostfix")));
                if (recycle != null)
                {
                    _feedingHarmony.Patch(recycle, prefix: new HarmonyMethod(t.GetMethod("RecyclePrefix")),
                                          postfix: new HarmonyMethod(t.GetMethod("RecyclePostfix")));
                }
                if (maybeAttack != null)
                {
                    _feedingHarmony.Patch(maybeAttack, prefix: new HarmonyMethod(t.GetMethod("AttackPrefix")),
                                          postfix: new HarmonyMethod(t.GetMethod("AttackPostfix")));
                }
                if (mouthEnter != null)
                    _feedingHarmony.Patch(mouthEnter, postfix: new HarmonyMethod(t.GetMethod("MouthEnterPostfix")));
                if (mouthExit != null)
                    _feedingHarmony.Patch(mouthExit, postfix: new HarmonyMethod(t.GetMethod("MouthExitPostfix")));
                if (mouthUpdate != null)
                    _feedingHarmony.Patch(mouthUpdate, postfix: new HarmonyMethod(t.GetMethod("MouthUpdatePostfix")));

                if (decayUpdate != null)
                {
                    _feedingHarmony.Patch(decayUpdate, prefix: new HarmonyMethod(t.GetMethod("DecayPrefix")),
                                          postfix: new HarmonyMethod(t.GetMethod("DecayPostfix")));
                }
                Log.LogInfo("feeding exact Harmony hooks installed");
            }
            catch (Exception e) { Log.LogWarning("feeding exact Harmony install failed: " + e.Message); _harmonyTried = true; }
        }

        // ---- lifecycle ------------------------------------------------------------------------------
        private void Awake()
        {
            Log = Logger;
            // Env switches let a capture run arm the hooks before the very first world load.
            UnityEngine.SceneManagement.SceneManager.sceneLoaded += OnSceneLoaded;
            TryInstallFeedingHarmony();
            _pauseOnLoad = Environment.GetEnvironmentVariable("BIBITES_DUMP_PAUSE_ON_LOAD") == "1";
            _dtPin = Environment.GetEnvironmentVariable("BIBITES_DUMP_LOCK_DT") == "1";
            _enabled = Config.Bind("Dump", "Enabled", true, "Run the parity dump HTTP server");
            _portCfg = Config.Bind("Dump", "Port", 17801, "Localhost TCP port (127.0.0.1 only)");
            if (!_enabled.Value) { Log.LogInfo("parity dump disabled in config"); return; }
            int port = _portCfg.Value;
            int envPort;
            var env = Environment.GetEnvironmentVariable("BIBITES_DUMP_PORT");
            if (!string.IsNullOrEmpty(env) && int.TryParse(env, out envPort) && envPort > 0 && envPort < 65536) port = envPort;
            _port = port;
            try
            {
                _listener = new HttpListener();
                _listener.Prefixes.Add("http://127.0.0.1:" + port + "/");
                _listener.Start();
                _running = true;
                _thread = new Thread(Loop) { IsBackground = true, Name = "ParityDump-listener" };
                _thread.Start();
                Log.LogInfo("parity dump listening on http://127.0.0.1:" + port + "/");
            }
            catch (Exception e)
            {
                Log.LogError("parity dump server failed to start: " + e);
                _listener = null;
            }
        }

        private void Update()
        {
            Action action;
            while (Queue.TryDequeue(out action)) action();
            PinTickDt();
            MaintainLoadHook();
            if (!_harmonyTried) TryInstallFeedingHarmony();
        }

        private void OnDestroy()
        {
            _running = false;
            try { UnityEngine.SceneManagement.SceneManager.sceneLoaded -= OnSceneLoaded; } catch { }
            try { if (_listener != null) { _listener.Stop(); _listener.Close(); } } catch { }
        }

        // ---- tiny HTTP server (Mono's managed HttpListener needs no URL ACL on 127.0.0.1) ----------------
        private void Loop()
        {
            while (_running)
            {
                HttpListenerContext ctx;
                try { ctx = _listener.GetContext(); }
                catch { if (!_running) return; continue; }
                ThreadPool.QueueUserWorkItem(_ => Handle(ctx));
            }
        }

        private void Handle(HttpListenerContext ctx)
        {
            int status = 200;
            string body;
            try
            {
                string method = ctx.Request.HttpMethod.ToUpperInvariant();
                string path = ctx.Request.Url.AbsolutePath;
                string text = null;
                if (ctx.Request.HasEntityBody)
                    using (var reader = new StreamReader(ctx.Request.InputStream, Encoding.UTF8)) text = reader.ReadToEnd();
                body = Dispatch(method, path, text);
            }
            catch (Exception e)
            {
                status = 500;
                body = Json.Write(new JObj().Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message));
                Log.LogError("dump request failed: " + e);
            }
            try
            {
                byte[] bytes = Encoding.UTF8.GetBytes(body ?? "{}");
                ctx.Response.StatusCode = status;
                ctx.Response.ContentType = "application/json";
                ctx.Response.ContentLength64 = bytes.Length;
                ctx.Response.OutputStream.Write(bytes, 0, bytes.Length);
                ctx.Response.OutputStream.Close();
            }
            catch (Exception e) { Log.LogWarning("could not write dump response: " + e.Message); }
        }

        private string Dispatch(string method, string path, string body)
        {
            switch (method + " " + path)
            {
                case "GET /d/ping":
                    return Json.Write(new JObj().Add("ok", true).Add("plugin", Version).Add("unity", Application.unityVersion).Add("game", Application.version));
                case "POST /d/timescale":
                {
                    var req = MiniJson.ParseObject(body ?? "{}");
                    float v = req.ContainsKey("value") ? Convert.ToSingle(req["value"], CultureInfo.InvariantCulture) : 1f;
                    Time.timeScale = v;
                    return Json.Write(new JObj().Add("ok", true).Add("timeScale", Time.timeScale));
                }
                case "GET /d/time":
                    return Json.Write(OnMain(TimeSnapshot));
                case "GET /d/pacing":
                    return Json.Write(OnMain(PacingSnapshot));
                case "POST /d/pacing":
                {
                    var req = MiniJson.ParseObject(body ?? "{}");
                    bool on = req.ContainsKey("lock") && MiniJson.Truthy(req["lock"]);
                    OnMain(() => { SetDtPin(on); return PacingSnapshot(); });
                    return Json.Write(OnMain(PacingSnapshot));
                }
                case "POST /d/load_pause":
                {
                    var req = MiniJson.ParseObject(body ?? "{}");
                    bool on = req.ContainsKey("enabled") && MiniJson.Truthy(req["enabled"]);
                    return Json.Write(OnMain(() => { SetLoadPause(on); return LoadPauseSnapshot(); }));
                }
                case "GET /d/load_pause":
                    return Json.Write(OnMain(LoadPauseSnapshot));
                case "GET /d/preload":
                    return Json.Write(OnMain(PreloadSnapshot));
                case "POST /d/preload":
                {
                    var values = MiniJson.ParseObject(body ?? "{}");
                    return Json.Write(OnMain(() =>
                    {
                        _preload.Clear();
                        foreach (var kv in values) _preload[kv.Key] = kv.Value;
                        ApplyPreloadSettings();
                        return PreloadSnapshot();
                    }));
                }
                case "GET /d/dump":
                    return Json.Write(OnMain(() => BuildDump()));
                case "GET /d/feeding":
                    return Json.Write(OnMain(FeedingSnapshot));
                case "GET /d/ui":
                    return Json.Write(OnMain(BuildUiRuntime));
                case "GET /d/control_dump":
                    return Json.Write(OnMain(ControlDump));
                case "POST /d/control_set":
                    return Json.Write(OnMain(() => ControlSet(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/press_label":
                    return Json.Write(OnMain(() => PressLabel(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/slider_set":
                    return Json.Write(OnMain(() => SliderSet(MiniJson.ParseObject(body ?? "{}"))));
                case "GET /d/digestion_brain":
                    return Json.Write(OnMain(DigestionBrain));
                case "GET /d/fov_digest":
                    return Json.Write(OnMain(FovDigest));
                case "GET /d/camera_controls":
                    return Json.Write(OnMain(CameraControls));
                case "GET /d/zone_material":
                    return Json.Write(OnMain(ZoneMaterials));
                case "GET /d/screen_capture":
                    return Json.Write(OnMain(ScreenCaptureJson));
                case "GET /d/menu_background":
                    return Json.Write(OnMain(() => MenuBackground(1920, 1080)));
                case "GET /d/menu_background_info":
                    return Json.Write(OnMain(MenuBackgroundInfo));
                case "POST /d/video_control":
                    return Json.Write(OnMain(() => VideoControl(MiniJson.ParseObject(body ?? "{}"))));
                case "GET /d/ui_geometry":
                    return Json.Write(OnMain(UiGeometry));
                case "GET /d/button_theme":
                    return Json.Write(OnMain(ButtonTheme));
                case "GET /d/font_assets":
                    return Json.Write(OnMain(FontAssets));
                case "GET /d/font_kerning":
                    return Json.Write(OnMain(FontKerning));
                case "GET /d/bodies":
                    return Json.Write(OnMain(BodiesInfo));
                case "POST /d/hide_fullscreen_graphics":
                    return Json.Write(OnMain(() => HideFullscreenGraphics(MiniJson.ParseObject(body ?? "{}"))));
                case "GET /d/ui_overlay_ui_only":
                    return Json.Write(OnMain(() => UiOverlayAll(1920, 1080, true)));
                case "GET /d/ui_overlay_all":
                    return Json.Write(OnMain(() => UiOverlayAll(1920, 1080)));
                case "GET /d/ui_overlay":
                    return Json.Write(OnMain(() => UiOverlayCapture(1920, 1080)));
                case "POST /d/select_bibite":
                    return Json.Write(OnMain(() => SelectBibite(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/press_button":
                    return Json.Write(OnMain(() => PressButton(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/trace_start":
                {
                    var req = MiniJson.ParseObject(body ?? "{}");
                    string tracePath = req.ContainsKey("path") ? Convert.ToString(req["path"], CultureInfo.InvariantCulture) : null;
                    FeedingPatches.TraceStart(tracePath);
                    return Json.Write(new JObj().Add("ok", true).Add("path", tracePath)
                        .Add("hooked", FeedingPatches.TraceHooked));
                }
                case "POST /d/body_spriter":
                    return Json.Write(OnMain(() => BodySpriter(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/body_scripts":
                    return Json.Write(OnMain(() => BodyScripts(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/controller_enabled":
                {
                    var req = MiniJson.ParseObject(body ?? "{}");
                    bool en = !req.ContainsKey("enabled") || MiniJson.Truthy(req["enabled"]);
                    int n = 0;
                    var list = new JArr();
                    foreach (var obj in Resources.FindObjectsOfTypeAll(typeof(MonoBehaviour)))
                    {
                        var mb = obj as MonoBehaviour;
                        if (mb == null || !mb.gameObject.scene.IsValid()) continue;
                        if (mb.GetType().Name != "ProceduralSpriteController") continue;
                        mb.enabled = en; n++;
                        list.Add(new JObj().Add("path", UiPath(mb.transform)).Add("enabled", mb.enabled));
                    }
                    return Json.Write(new JObj().Add("ok", true).Add("enabled", en).Add("count", n).Add("controllers", list));
                }
                case "POST /d/normalize_ui":
                    return Json.Write(OnMain(() => NormalizeUi(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/activate":
                    return Json.Write(OnMain(() => ActivatePath(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/invoke":
                    return Json.Write(OnMain(() => InvokeOn(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/procedural_info":
                    return Json.Write(OnMain(() => ProceduralInfo(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/open_species":
                    return Json.Write(OnMain(() => OpenSpecies(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/panel_ints":
                    return Json.Write(OnMain(() => PanelInts(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/blit_material":
                    return Json.Write(OnMain(() => BlitMaterial(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/panel_material":
                    return Json.Write(OnMain(() => PanelMaterial(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/material_dump":
                    return Json.Write(OnMain(() => MaterialDump(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/material_float":
                    return Json.Write(OnMain(() => MaterialFloat(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/sprite_dump":
                    return Json.Write(OnMain(() => SpriteDump(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/object_renderers":
                    return Json.Write(OnMain(() => ObjectRenderers(MiniJson.ParseObject(body ?? "{}"))));
                case "POST /d/renderers":
                    return Json.Write(OnMain(() => RendererControl(MiniJson.ParseObject(body ?? "{}"))));
                case "GET /d/bibite_layers":
                    return Json.Write(OnMain(BibiteLayers));
                case "POST /d/trace_stop":
                {
                    int calls = FeedingPatches.TraceCallCount;
                    int ccalls = FeedingPatches.CollisionCallCount;
                    int ccontacts = FeedingPatches.CollisionContactCount;
                    string err = FeedingPatches.TraceError;
                    FeedingPatches.TraceStop();
                    return Json.Write(new JObj().Add("ok", true).Add("calls", calls).Add("error", err)
                        .Add("collision_calls", ccalls).Add("collision_contacts", ccontacts));
                }
                default:
                    return Json.Write(new JObj().Add("ok", false).Add("error", "no route " + method + " " + path));
            }
        }

        private static JObj TimeSnapshot()
        {
            var o = new JObj();
            o.Add("fixedDeltaTime", Time.fixedDeltaTime);
            o.Add("maximumDeltaTime", Time.maximumDeltaTime);
            o.Add("timeScale", Time.timeScale);
            o.Add("fixedTime", Time.fixedTime);
            o.Add("realtimeSinceStartup", Time.realtimeSinceStartup);
            AddEngine(o);
            return o;
        }

        private JObj PacingSnapshot()
        {
            var o = new JObj();
            o.Add("lock", _dtPin);
            o.Add("baseFixedDeltaTime", ReadBaseFixedDeltaTime());
            o.Add("fixedDeltaTime", Time.fixedDeltaTime);
            o.Add("maximumDeltaTime", Time.maximumDeltaTime);
            o.Add("timeScale", Time.timeScale);
            return o;
        }

        private static void AddEngine(JObj o)
        {
            try
            {
                var tc = FindType("ManagementScripts.TimeController");
                if (tc == null) { o.Add("engineTimeScale", null); return; }
                var etsField = tc.GetField("engineTimeScale", BindingFlags.Static | BindingFlags.Public);
                if (etsField == null) { o.Add("engineTimeScale", null); return; }
                var ets = etsField.GetValue(null);
                o.Add("engineTimeScale", F(PropVal(ets, "val")));
                var pausedField = tc.GetField("paused", BindingFlags.Static | BindingFlags.Public);
                o.Add("paused", pausedField != null ? (object)Convert.ToBoolean(pausedField.GetValue(null), CultureInfo.InvariantCulture) : null);
                var tk = FindType("OneUseScripts.TimeKeeper");
                if (tk != null)
                {
                    var st = tk.GetField("simulatedTime", BindingFlags.Static | BindingFlags.Public);
                    if (st != null) o.Add("simulatedTime", Convert.ToDouble(st.GetValue(null), CultureInfo.InvariantCulture));
                    var fps = tk.GetField("fps", BindingFlags.Static | BindingFlags.Public);
                    if (fps != null) o.Add("fps", F(fps.GetValue(null)));
                }
            }
            catch (Exception e) { o.Add("engineError", e.Message); }
        }

        // =============================================================================================
        // Reflection helpers
        // =============================================================================================
        private const BindingFlags InstanceFlags = BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic;
        private const BindingFlags StaticFlags = BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic;

        internal static Type FindType(string fullName)
        {
            foreach (var asm in AppDomain.CurrentDomain.GetAssemblies())
            {
                try
                {
                    var t = asm.GetType(fullName, false);
                    if (t != null) return t;
                }
                catch { }
            }
            return null;
        }

        private static object Static(object typeOrNull, string field)
        {
            try { return typeOrNull == null ? null : ((Type)typeOrNull).GetField(field, StaticFlags)?.GetValue(null); }
            catch { return null; }
        }

        private static object Inst(object instance, string field)
        {
            try
            {
                if (instance == null) return null;
                for (var t = instance.GetType(); t != null; t = t.BaseType)
                {
                    var f = t.GetField(field, InstanceFlags);
                    if (f != null) return f.GetValue(instance);
                }
            }
            catch { }
            return null;
        }

        private static object PropVal(object instance, string prop)
        {
            try
            {
                if (instance == null) return null;
                var p = instance.GetType().GetProperty(prop, InstanceFlags);
                return p != null && p.CanRead ? p.GetValue(instance) : null;
            }
            catch { return null; }
        }

        private static float F(object v) { try { return Convert.ToSingle(v, CultureInfo.InvariantCulture); } catch { return float.NaN; } }
        private static int I(object v) { try { return Convert.ToInt32(v, CultureInfo.InvariantCulture); } catch { return 0; } }
        private static bool B(object v) { try { return Convert.ToBoolean(v, CultureInfo.InvariantCulture); } catch { return false; } }

        private static List<object> AsList(object o)
        {
            var list = new List<object>();
            var en = o as IEnumerable;
            if (en == null) return list;
            foreach (var x in en) list.Add(x);
            return list;
        }

        private static bool IsSimple(Type t)
        {
            return t.IsPrimitive || t.IsEnum || t == typeof(string) || t == typeof(decimal)
                || t == typeof(Vector2) || t == typeof(Vector3) || t == typeof(Vector4) || t == typeof(Color)
                || t == typeof(Rect) || t == typeof(Bounds) || t == typeof(Quaternion);
        }

        private static object SimpleValue(object v)
        {
            if (v == null) return null;
            var t = v.GetType();
            if (t.IsEnum) return v.ToString();
            if (v is Vector2) { var x = (Vector2)v; return new JObj().Add("x", x.x).Add("y", x.y); }
            if (v is Vector3) { var x = (Vector3)v; return new JObj().Add("x", x.x).Add("y", x.y).Add("z", x.z); }
            if (v is Vector4) { var x = (Vector4)v; return new JObj().Add("x", x.x).Add("y", x.y).Add("z", x.z).Add("w", x.w); }
            if (v is Color) { var x = (Color)v; return new JObj().Add("r", x.r).Add("g", x.g).Add("b", x.b).Add("a", x.a); }
            if (v is Rect) { var x = (Rect)v; return new JObj().Add("x", x.x).Add("y", x.y).Add("w", x.width).Add("h", x.height); }
            if (v is Bounds) { var x = (Bounds)v; return new JObj().Add("center", x.center).Add("size", x.size); }
            if (v is Quaternion) { var x = (Quaternion)v; return new JObj().Add("x", x.x).Add("y", x.y).Add("z", x.z).Add("w", x.w); }
            return v;
        }

        // =============================================================================================
        // The dump
        // =============================================================================================
        private JObj BuildDump()
        {
            var root = new JObj();
            root.Add("schema", "rustibites.parity.prefab_dump.v1");
            root.Add("createdAtUtc", DateTime.UtcNow.ToString("o", CultureInfo.InvariantCulture));
            root.Add("plugin", Version);
            root.Add("gameVersion", Application.version);
            root.Add("unityVersion", Application.unityVersion);
            root.Add("platform", Application.platform.ToString());
            root.Add("scene", UnityEngine.SceneManagement.SceneManager.GetActiveScene().name);
            root.Add("source", "runtime reflection over the loaded game assemblies + scene/prefab objects; nothing patched, nothing written");
            root.Add("warning", "Values are the RUNTIME defaults of the installed build. Material *settings* (plant/meat/fat/armor) may have been replaced by a loaded scenario/save; `materials.default_*` are the untouched prefab (ScriptableObject) values.");

            root.Add("matter_materials", MatterMaterials());
            root.Add("physics2d", PhysicsConfig());
            root.Add("prefabs", Prefabs());
            root.Add("physics_materials", PhysicsMaterials());
            root.Add("physics2d_global_scratch", PhysicsScratch());
            return root;
        }

        private JObj MatterMaterials()
        {
            var o = new JObj();
            o.Add("note", "MatterMaterial prefab assets (DefaultMaterials = the un-instantiated originals) and the live MaterialSettings. EnergyDensity/HealthDensity for Energy/Health/Fat/Armor exist only here.");
            var mm = FindType("SimulationScripts.MatterMaterialManager");
            var matType = FindType("SimulationScripts.MatterMaterial");
            if (mm == null || matType == null)
            {
                o.Add("error", "MatterMaterialManager/MatterMaterial type not found - is a simulation world loaded?");
                return o;
            }
            var inst = Static(mm, "Instance");
            var defaults = AsList(Static(mm, "DefaultMaterials"));
            var arr = new JArr();
            foreach (var m in defaults) arr.Add(Material(m, matType, "default"));
            o.Add("default_materials", arr);

            var serialized = new JArr();
            foreach (var name in new[] { "plant", "meat", "fat", "armor", "energy", "health", "none" })
            {
                var m = Inst(inst, name);
                if (m != null) serialized.Add(Material(m, matType, "prefab_" + name));
            }
            o.Add("manager_serialized_materials", serialized);

            var live = new JArr();
            foreach (var name in new[] { "Plant", "Meat", "Fat", "Armor", "Energy", "Health", "None" })
            {
                var m = Static(mm, name);
                if (m != null) live.Add(Material(m, matType, "live_" + name));
            }
            o.Add("live_materials", live);

            // matter material settings (what the scenario/save can change): every FloatSetting/BoolSetting .val
            var settings = AsList(Static(mm, "MaterialSettings"));
            var sa = new JArr();
            foreach (var s in settings)
            {
                if (s == null) continue;
                sa.Add(DumpSerializedObject(s, 0));
            }
            o.Add("matter_material_settings", sa);
            return o;
        }

        private static JObj Material(object m, Type matType, string tag)
        {
            var o = new JObj();
            o.Add("tag", tag);
            o.Add("type", m == null ? null : m.GetType().Name);
            if (m == null) return o;
            foreach (var f in matType.GetFields(BindingFlags.Instance | BindingFlags.Public))
            {
                if (f.IsStatic) continue;
                var v = f.GetValue(m);
                if (SimpleValue(v) != null && IsSimple(f.FieldType)) o.Add(f.Name, SimpleValue(v));
            }
            o.Add("NormalEnergyAtScale1", F(PropVal(m, "NormalEnergyAtScale1")));
            var constAmount = matType.GetField("NormalAmountAtScale1", BindingFlags.Static | BindingFlags.Public);
            if (constAmount != null) o.Add("NormalAmountAtScale1", F(constAmount.GetValue(null)));
            var constRadius = matType.GetField("NormalRadiusAtScale1", BindingFlags.Static | BindingFlags.Public);
            if (constRadius != null) o.Add("NormalRadiusAtScale1", F(constRadius.GetValue(null)));
            return o;
        }

        private JObj PhysicsConfig()
        {
            var o = new JObj();
            o.Add("note", "Physics2D global configuration of the installed build (Unity " + Application.unityVersion + ")");
            o.Add("gravity", SimpleValue(Physics2D.gravity));
            o.Add("properties", DumpStaticProps(typeof(Physics2D)));
            o.Add("layer_names", LayerNames());
            o.Add("ignore_layer_collision_pairs", IgnorePairs());
            o.Add("ignore_layer_collision_matrix", IgnoreMatrix());
            o.Add("time", new JObj().Add("fixedDeltaTime", Time.fixedDeltaTime).Add("maximumDeltaTime", Time.maximumDeltaTime).Add("timeScale", Time.timeScale));
            o.Add("quality", new JObj().Add("vSyncCount", QualitySettings.vSyncCount).Add("targetFrameRate", Application.targetFrameRate).Add("antiAliasing", QualitySettings.antiAliasing));
            return o;
        }

        private static JObj DumpStaticProps(Type t)
        {
            var o = new JObj();
            foreach (var p in t.GetProperties(BindingFlags.Public | BindingFlags.Static))
            {
                if (!p.CanRead || p.GetIndexParameters().Length > 0) continue;
                try
                {
                    var v = p.GetValue(null);
                    if (v == null) continue;
                    var vt = v.GetType();
                    if (vt.IsEnum || vt.IsPrimitive || vt == typeof(string)) o.Add(p.Name, SimpleValue(v));
                    else if (IsSimple(vt)) o.Add(p.Name, SimpleValue(v));
                }
                catch { }
            }
            return o;
        }

        private static JArr LayerNames()
        {
            var a = new JArr();
            for (int i = 0; i < 32; i++) a.Add(new JObj().Add("index", i).Add("name", LayerMask.LayerToName(i)));
            return a;
        }

        private static JArr IgnorePairs()
        {
            var a = new JArr();
            for (int i = 0; i < 32; i++)
                for (int j = i + 1; j < 32; j++)
                    if (Physics2D.GetIgnoreLayerCollision(i, j)) a.Add(new JArr().Add(i).Add(j));
            return a;
        }

        private static JArr IgnoreMatrix()
        {
            // 32 strings of 32 chars, '1' = collisions ignored between layer i (row) and layer j (column)
            var a = new JArr();
            for (int i = 0; i < 32; i++)
            {
                var sb = new StringBuilder(32);
                for (int j = 0; j < 32; j++)
                    sb.Append(Physics2D.GetIgnoreLayerCollision(i, j) ? '1' : '0');
                a.Add(sb.ToString());
            }
            return a;
        }

        private JObj PhysicsScratch()
        {
            var o = new JObj();
            // Built-in Physics2D static *fields* (some Unity versions expose settings as fields rather than properties).
            foreach (var f in typeof(Physics2D).GetFields(BindingFlags.Public | BindingFlags.Static))
            {
                try
                {
                    var v = f.GetValue(null);
                    if (v == null) continue;
                    if (v.GetType().IsPrimitive || v.GetType().IsEnum || IsSimple(v.GetType())) o.Add(f.Name, SimpleValue(v));
                }
                catch { }
            }
            return o;
        }

        private static readonly string[] PrefabFields = {
            "bibiteEntry", "eggEntry", "plantEntry", "meatEntry", "pelletSpawner", "pelletSpawnPoint",
            "bibiteSpawner", "pheromonesSource", "colorKiller", "pheroTower", "radTower", "radioZone",
        };

        private JObj Prefabs()
        {
            var o = new JObj();
            var ws = FindType("ManagementScripts.WorldObjectsSpawner");
            if (ws == null) { o.Add("error", "WorldObjectsSpawner type not found - is a simulation world loaded?"); return o; }
            var inst = Static(ws, "Instance");
            if (inst == null) { o.Add("error", "WorldObjectsSpawner.Instance is null - load a world first"); return o; }
            var arr = new JArr();
            foreach (var name in PrefabFields)
            {
                var go = Inst(inst, name) as GameObject;
                var entry = new JObj().Add("field", name);
                if (go == null) entry.Add("prefab", null);
                else
                {
                    entry.Add("prefab", DumpGameObject(go, 0, 3));
                }
                arr.Add(entry);
            }
            o.Add("spawner_prefabs", arr);

            // Other scene singletons that hold prefabs / serialized defaults useful to body & world code.
            var singletons = new JArr();
            foreach (var pair in new (string Type, string Field)[] {
                ("ManagementScripts.SimulationManager", null),
                ("ManagementScripts.ZoneManager", "instance"),
                ("OneUseScripts.TimeKeeper", null),
                ("SimulationScripts.TagsManager", "instance"),
            })
            {
                var t = FindType(pair.Type);
                if (t == null) continue;
                var v = pair.Field == null ? (Static(t, "Instance") ?? Static(t, "instance")) : (Static(t, pair.Field) ?? Static(t, "Instance"));
                if (v == null) continue;
                // only dump serialized constants of the manager itself (not the whole scene graph)
                singletons.Add(DumpSerializedObject(v, 1));
            }
            o.Add("scene_singletons", singletons);
            return o;
        }

        private static readonly Dictionary<string, PhysicsMaterial2D> PhysicsMaterialCache = new Dictionary<string, PhysicsMaterial2D>();

        private static JObj PhysicsMaterialDump(PhysicsMaterial2D pm)
        {
            var o = new JObj();
            o.Add("name", pm.name);
            o.Add("friction", pm.friction);
            o.Add("bounciness", pm.bounciness);
            try { o.Add("frictionCombine", pm.frictionCombine.ToString()); } catch { }
            try { o.Add("bounceCombine", pm.bounceCombine.ToString()); } catch { }
            return o;
        }

        /// <summary>Every PhysicsMaterial2D the prefabs reference (friction / restitution live only there).</summary>
        private JObj PhysicsMaterials()
        {
            var o = new JObj();
            var arr = new JArr();
            foreach (var kv in PhysicsMaterialCache.OrderBy(k => k.Key, StringComparer.Ordinal)) arr.Add(PhysicsMaterialDump(kv.Value));
            o.Add("referenced_by_prefabs", arr);
            o.Add("note", "PhysicsMaterial2D assets are not saved in worlds; they are the friction/restitution source for bibite/pellet colliders.");
            return o;
        }
        private JObj DumpGameObject(GameObject go, int depth, int maxDepth)
        {
            var o = new JObj();
            o.Add("name", go.name);
            o.Add("tag", go.tag);
            o.Add("layer", go.layer);
            o.Add("layerName", LayerMask.LayerToName(go.layer));
            o.Add("activeSelf", go.activeSelf);
            var comps = new JArr();
            foreach (var c in go.GetComponents<Component>())
            {
                if (c == null) { comps.Add(new JObj().Add("type", "MissingScript")); continue; }
                comps.Add(DumpComponent(c));
            }
            o.Add("components", comps);
            var children = new JArr();
            if (depth < maxDepth)
                foreach (Transform child in go.transform)
                    children.Add(DumpGameObject(child.gameObject, depth + 1, maxDepth));
            else if (go.transform.childCount > 0)
                o.Add("children_omitted", go.transform.childCount);
            o.Add("children", children);
            return o;
        }

        private JObj DumpComponent(Component c)
        {
            var o = new JObj();
            var t = c.GetType();
            o.Add("type", t.FullName ?? t.Name);
            if (c is Rigidbody2D)
            {
                var rb = (Rigidbody2D)c;
                var r = new JObj();
                r.Add("bodyType", rb.bodyType.ToString());
                r.Add("gravityScale", rb.gravityScale);
                r.Add("mass", rb.mass);
                r.Add("useAutoMass", rb.useAutoMass);
                r.Add("linearDamping", rb.linearDamping);
                r.Add("angularDamping", rb.angularDamping);
                r.Add("interpolation", rb.interpolation.ToString());
                r.Add("collisionDetectionMode", rb.collisionDetectionMode.ToString());
                r.Add("constraints", rb.constraints.ToString());
                r.Add("freezeRotation", rb.freezeRotation);
                r.Add("simulated", rb.simulated);
                r.Add("linearVelocity", SimpleValue(rb.linearVelocity));
                r.Add("angularVelocity", rb.angularVelocity);
                try { r.Add("inertia", rb.inertia); } catch { }
                try { r.Add("sleepMode", rb.sleepMode.ToString()); } catch { }
                o.Add("rigidbody2d", r);
            }
            else if (c is Collider2D)
            {
                var col = (Collider2D)c;
                var r = new JObj();
                r.Add("colliderType", t.Name);
                r.Add("isTrigger", col.isTrigger);
                r.Add("usedByComposite", col.usedByComposite);
                r.Add("usedByEffector", col.usedByEffector);
                r.Add("offset", SimpleValue(col.offset));
                try { r.Add("density", col.density); } catch { }
                try { var er = PropVal(col, "edgeRadius"); if (er != null) r.Add("edgeRadius", F(er)); } catch { }
                try { var co = PropVal(col, "compositeOperation"); if (co != null) r.Add("compositeOperation", co.ToString()); } catch { }
                if (col is CapsuleCollider2D) { var cc = (CapsuleCollider2D)col; r.Add("size", SimpleValue(cc.size)); r.Add("direction", cc.direction.ToString()); }
                if (col is BoxCollider2D) { var bc = (BoxCollider2D)col; r.Add("size", SimpleValue(bc.size)); }
                if (col is CircleCollider2D) { var cc = (CircleCollider2D)col; r.Add("radius", cc.radius); }
                try
                {
                    if (col.sharedMaterial != null)
                    {
                        r.Add("sharedMaterial", col.sharedMaterial.name);
                        var pm = col.sharedMaterial as PhysicsMaterial2D;
                        if (pm != null)
                        {
                            r.Add("physics_material2d", PhysicsMaterialDump(pm));
                            if (!PhysicsMaterialCache.ContainsKey(pm.name)) PhysicsMaterialCache[pm.name] = pm;
                        }
                    }
                }
                catch { }
                o.Add("collider2d", r);
            }
            var fields = DumpSerializedFields(c, 0);
            if (fields.Count > 0) o.Add("serialized_fields", fields);
            return o;
        }

        /// <summary>All public instance fields and [SerializeField] private fields of a component, as simple values.</summary>
        private JObj DumpSerializedFields(object obj, int depth)
        {
            var o = new JObj();
            if (obj == null || depth > 2) return o;
            var type = obj.GetType();
            foreach (var f in type.GetFields(InstanceFlags))
            {
                if (f.IsStatic || f.IsInitOnly) continue;
                if (!f.IsPublic && !f.IsDefined(typeof(SerializeField), true)) continue;
                if (f.IsDefined(typeof(NonSerializedAttribute), true)) continue;
                try
                {
                    var v = f.GetValue(obj);
                    var fv = DumpValue(v, f.FieldType, depth);
                    if (fv != null) o.Add(f.Name, fv);
                }
                catch { }
            }
            return o;
        }

        private object DumpValue(object v, Type declared, int depth)
        {
            if (v == null) return null;
            var t = v.GetType();
            if (IsSimple(t)) return SimpleValue(v);
            if (t == typeof(LayerMask)) return LayerMaskDump((LayerMask)v);
            if (typeof(UnityEngine.Events.UnityEventBase).IsAssignableFrom(t)) return new JObj().Add("type", t.Name);
            // *Setting values (FloatSetting/IntSetting/BoolSetting/FloatUserSetting/...): dump the value + range, not their event plumbing.
            if (t.Name.EndsWith("Setting", StringComparison.Ordinal) && !(v is UnityEngine.Object))
                return SettingDump(v);
            if (v is UnityEngine.Object uo) return new JObj().Add("ref", string.IsNullOrEmpty(uo.name) ? "(unnamed)" : uo.name).Add("type", t.Name);
            if (v is Array arr)
            {
                var et = t.GetElementType();
                var a = new JArr();
                int n = 0;
                foreach (var x in arr) { if (n++ >= 64) break; a.Add(DumpValue(x, et, depth + 1)); }
                return new JObj().Add("array", a).Add("length", arr.Length);
            }
            if (v is IList list && t.IsGenericType)
            {
                var et = t.GetGenericArguments()[0];
                var a = new JArr();
                int n = 0;
                foreach (var x in list) { if (n++ >= 64) break; a.Add(DumpValue(x, et, depth + 1)); }
                return new JObj().Add("list", a).Add("count", list.Count);
            }
            if (depth < 2) return DumpSerializedObject(v, depth + 1);
            return new JObj().Add("type", t.Name);
        }

        private static JObj LayerMaskDump(LayerMask mask)
        {
            var names = new JArr();
            int value = mask.value;
            for (int i = 0; i < 32; i++) if ((value & (1 << i)) != 0) names.Add(i + ":" + LayerMask.LayerToName(i));
            return new JObj().Add("value", value).Add("layers", names);
        }

        private static JObj SettingDump(object setting)
        {
            var o = new JObj();
            o.Add("type", setting.GetType().Name);
            o.Add("val", SimpleValue(PropVal(setting, "val")));
            var def = PropVal(setting, "DefaultValue");
            if (def != null) o.Add("default", SimpleValue(def));
            var min = PropVal(setting, "minValue");
            if (min != null) o.Add("min", SimpleValue(min));
            var max = PropVal(setting, "maxValue");
            if (max != null) o.Add("max", SimpleValue(max));
            var name = PropVal(setting, "Name");
            if (name is string s && s.Length > 0) o.Add("name", s);
            return o;
        }

        private JObj DumpSerializedObject(object obj, int depth)
        {
            var o = new JObj();
            if (obj == null) return o;
            var t = obj.GetType();
            o.Add("type", t.Name);
            if (depth == 0 || !(obj is UnityEngine.Object))
            {
                var fields = DumpSerializedFields(obj, depth);
                if (fields.Count > 0) o.Add("fields", fields);
            }
            else
            {
                var fields = DumpSerializedFields(obj, depth);
                if (fields.Count > 0) o.Add("fields", fields);
            }
            return o;
        }
        // ---- feeding snapshot (task-94) -------------------------------------------------------------
        // Game-side feeding evidence without patching anything: current per-bibite stomach contents,
        // held pellets, mouth bite timers/counters, energy/health, and per-zone pellet stock/production
        // counters. Capture-side diffs of successive snapshots give per-material intake (a successful
        // bite/swallow proxy) and pellet-mass balance. No game type is referenced at compile time.
        private JObj FeedingSnapshot()
        {
            var root = new JObj();
            root.Add("schema", "rustibites.parity.feeding.v1");
            root.Add("t", GetSimulatedTime());
            root.Add("plugin", Version);
            root.Add("gameVersion", Application.version);
            try
            {
                var bibiteType = FindType("SimulationScripts.BibiteScripts.BibiteBody");
                var pelletType = FindType("SimulationScripts.MatterPellet");
                var zoneType = FindType("SimulationScripts.Zone");
                var bodies = bibiteType == null ? new UnityEngine.Object[0] : UnityEngine.Object.FindObjectsOfType(bibiteType);
                var zones = zoneType == null ? new UnityEngine.Object[0] : UnityEngine.Object.FindObjectsOfType(zoneType);

                var bibites = new JArr();
                var matProps = new Dictionary<string, JObj>();
                foreach (var b in bodies)
                {
                    object body = b;
                    var idObj = Inst(body, "id");
                    int id = I(Inst(idObj, "id"));
                    if (id == 0) id = b.GetInstanceID();
                    var energy = Inst(body, "Energy");
                    var health = Inst(body, "Health");
                    var stomach = Inst(body, "stomach");
                    var mouth = Inst(body, "mouth");
                    var o = new JObj()
                        .Add("id", id)
                        .Add("energy", F(Inst(energy, "Amount")))
                        .Add("health", F(Inst(health, "Amount")))
                        .Add("d2_size", F(Inst(body, "d2Size")))
                        .Add("stomach_total", F(PropVal(stomach, "totalAmount")))
                        .Add("n_stomach", I(Inst(stomach, "nContent")))
                        .Add("n_held", I(Inst(mouth, "nHeld")))
                        .Add("n_in_mouth", FeedingPatches.NMouth(mouth))
                        .Add("mouth_targets", FeedingPatches.MouthTargets(mouth))
                        .Add("mouth_contact_ticks", FeedingPatches.ContactValue(mouth, "contact"))
                        .Add("plant_contact_ticks", FeedingPatches.ContactValue(mouth, "plant"))
                        .Add("mouth_contact_enter_events", FeedingPatches.ContactValue(mouth, "enter"))
                        .Add("first_mouth_contact_tick", FeedingPatches.ContactValue(mouth, "first"))
                        .Add("first_plant_contact_tick", FeedingPatches.ContactValue(mouth, "first_plant"))
                        .Add("bite_progress", F(Inst(mouth, "biteProgress")))
                        .Add("bite_period", F(Inst(mouth, "bitePeriod")))
                        .Add("bibites_bitten", I(Inst(mouth, "bibitesBitten")))
                        .Add("damage_dealt", F(Inst(mouth, "totalDamageDealt")))
                        .Add("total_murders", I(Inst(mouth, "totalMurders")))
                        .Add("murdered_area", F(Inst(mouth, "murderedArea")));
                    var tr = Inst(body, "transform") as Transform;
                    if (tr != null) o.Add("pos", new JObj().Add("x", tr.position.x).Add("y", tr.position.y));
                    // stomach contents per material: the raw intake signal
                    var mats = new Dictionary<string, float>();
                    var contents = Inst(stomach, "stomachContents") as Array;
                    int nContent = arrLength(contents);
                    for (int i = 0; i < nContent; i++)
                    {
                        var sc = contents.GetValue(i);
                        var matObj = Inst(sc, "material");
                        string mat = MaterialName(matObj);
                        AddMaterialProps(matProps, matObj, mat);
                        float amt = F(Inst(sc, "amount"));
                        mats[mat] = (mats.TryGetValue(mat, out var prev) ? prev : 0f) + amt;
                    }
                    var mj = new JObj();
                    foreach (var kv in mats) mj.Add(kv.Key, kv.Value);
                    o.Add("stomach_materials", mj);
                    // pellets currently held in the mouth (private array read by reflection)
                    var held = new JArr();
                    var objs = Inst(mouth, "objectsInMouth") as Array;
                    if (objs != null && pelletType != null)
                    {
                        for (int i = 0; i < objs.Length; i++)
                        {
                            var go = objs.GetValue(i) as GameObject;
                            if (go == null) continue;
                            var mp = go.GetComponent(pelletType);
                            if (mp == null) continue;
                            var hmat = Inst(mp, "material");
                            string hname = MaterialName(hmat);
                            AddMaterialProps(matProps, hmat, hname);
                            held.Add(new JObj()
                                .Add("amount", F(PropVal(mp, "amount")))
                                .Add("material", hname)
                                .Add("held", B(Inst(mp, "held"))));
                        }
                    }
                    o.Add("held_pellets", held);
                    bibites.Add(o);
                }
                root.Add("bibites", bibites);

                var zarr = new JArr();
                double plantAmount = 0.0, meatAmount = 0.0;
                int pelletCount = 0;
                foreach (var z in zones)
                {
                    var zp = new JObj()
                        .Add("pellet_biomass", F(Inst(z, "pelletBiomass")))
                        .Add("max_biomass", F(Inst(z, "maxBiomass")))
                        .Add("free_biomass", F(PropVal(z, "freeBiomass")))
                        .Add("pellet_production", F(Inst(z, "pelletProduction")))
                        .Add("pellet_progress", F(Inst(z, "pelletProgress")))
                        .Add("radius", F(Inst(z, "radius")));
                    var zpos = Inst(z, "pos");
                    if (zpos != null) zp.Add("pos", SimpleValue(zpos));
                    var pellets = AsList(Inst(z, "zonePellets"));
                    double zpAmount = 0.0, zmAmount = 0.0;
                    foreach (var p in pellets)
                    {
                        var pmat = Inst(p, "material");
                        string mat = MaterialName(pmat);
                        AddMaterialProps(matProps, pmat, mat);
                        float amt = F(PropVal(p, "amount"));
                        if (mat.Contains("plant")) zpAmount += amt;
                        else if (mat.Contains("meat")) zmAmount += amt;
                    }
                    plantAmount += zpAmount; meatAmount += zmAmount; pelletCount += pellets.Count;
                    zp.Add("pellets_n", pellets.Count).Add("plant_amount", zpAmount).Add("meat_amount", zmAmount);
                    zarr.Add(zp);
                }
                // cumulative pellet-energy census: monotonic produced/removed terms per zone. Polling
                // cannot separate zone recycle from decay vs bites, so recycled/decayed are null and
                // culled_energy == removed residual; exact accumulators are the next plugin step.
                var current = new Dictionary<int, double>();
                var currentZone = new Dictionary<int, int>();
                int zidx = 0;
                foreach (var z in zones)
                {
                    foreach (var p in AsList(Inst(z, "zonePellets")))
                    {
                        if (p == null) continue;
                        int id = ((UnityEngine.Object)p).GetInstanceID();
                        double e = F(PropVal(p, "energy"));
                        if (e < 0) e = 0;
                        current[id] = e;
                        currentZone[id] = zidx;
                        FeedingPatches.PelletZone[id] = zidx;
                    }
                    zidx++;
                }
                while (_zoneProducedEnergy.Count < zones.Length) { _zoneProducedEnergy.Add(0.0); _zoneRemovedEnergy.Add(0.0); }
                if (!_censusInitialised)
                {
                    foreach (var kv in current) { _censusInitialEnergy += kv.Value; _pelletEnergyCensus[kv.Key] = kv.Value; _pelletZoneCensus[kv.Key] = currentZone[kv.Key]; }
                    _censusInitialised = true;
                    FeedingPatches.ResetExact(); // initial energy covers everything up to this first poll
                }
                else
                {
                    var zoneProduced = new double[Math.Max(1, zones.Length)];
                    var zoneRemoved = new double[Math.Max(1, zones.Length)];
                    foreach (var kv in current)
                    {
                        if (!_pelletEnergyCensus.ContainsKey(kv.Key))
                        {
                            _censusProducedEnergy += kv.Value;
                            zoneProduced[Math.Min(currentZone[kv.Key], zoneProduced.Length - 1)] += kv.Value;
                        }
                    }
                    foreach (var kv in _pelletEnergyCensus)
                    {
                        if (!current.ContainsKey(kv.Key))
                        {
                            _censusRemovedEnergy += kv.Value;
                            int zz = _pelletZoneCensus.TryGetValue(kv.Key, out var z2) ? Math.Min(z2, zoneRemoved.Length - 1) : 0;
                            zoneRemoved[zz] += kv.Value;
                        }
                    }
                    for (int k = 0; k < _zoneProducedEnergy.Count; k++)
                    {
                        _zoneProducedEnergy[k] += k < zoneProduced.Length ? zoneProduced[k] : 0.0;
                        _zoneRemovedEnergy[k] += k < zoneRemoved.Length ? zoneRemoved[k] : 0.0;
                    }
                    _pelletEnergyCensus.Clear();
                    _pelletZoneCensus.Clear();
                    foreach (var kv in current) { _pelletEnergyCensus[kv.Key] = kv.Value; _pelletZoneCensus[kv.Key] = currentZone[kv.Key]; }
                }
                double currentEnergy = 0.0;
                foreach (var kv in current) currentEnergy += kv.Value;
                double consumedEst = _censusInitialEnergy + _censusProducedEnergy - currentEnergy - _censusRemovedEnergy;
                root.Add("counters", new JObj()
                    .Add("initial_energy", _censusInitialEnergy)
                    .Add("produced_energy_cumulative", _censusProducedEnergy)
                    .Add("removed_energy_cumulative", _censusRemovedEnergy)
                    .Add("culled_energy_cumulative", _censusRemovedEnergy)
                    .Add("recycled_energy_cumulative", null)
                    .Add("decayed_energy_cumulative", null)
                    .Add("consumed_energy_estimate_cumulative", consumedEst)
                    .Add("current_pellet_energy", currentEnergy)
                    .Add("produced_energy_exact", FeedingPatches.Produced)
                    .Add("consumed_energy_exact", FeedingPatches.Consumed)
                    .Add("recycled_energy_exact", FeedingPatches.Recycled)
                    .Add("decayed_energy_exact", FeedingPatches.Decayed)
                    .Add("culled_energy_exact", FeedingPatches.Culled)
                    .Add("conservation_residual_energy",
                         _censusInitialEnergy + FeedingPatches.Produced - FeedingPatches.Consumed
                         - FeedingPatches.Decayed - FeedingPatches.Recycled - FeedingPatches.Culled - currentEnergy)
                    .Add("evidence_exact", "harmony_exact")
                    .Add("note_exact", "Harmony hooks: produced = SpawnAPellet returned energy; consumed/recycled/decayed = pellet energy removed outside/inside Zone.RecyclePellets/MatterDecayProcessor.FixedUpdate. culled_energy is 0 today (all destroy paths are amount-based); residual ~0 modulo polls.")
                    .Add("mouth_contact_ticks_total", FeedingPatches.ContactTotal("contact"))
                    .Add("plant_contact_ticks_total", FeedingPatches.ContactTotal("plant"))
                    .Add("mouth_contact_enter_events_total", FeedingPatches.ContactTotal("enter"))
                    .Add("evidence", "polling_census_proxy")
                    .Add("missing_terms", new JArr().Add("recycled_energy").Add("decayed_energy"))
                    .Add("note", "polling census: produced = new pellet energy; removed/culled = residual energy of pellets gone between polls; recycled/decayed cannot be split by polling and are null; consumed_energy_estimate = initial + produced - current - removed. Exact accumulators are the task-94 appendix-D follow-up."));
                for (int k = 0; k < zarr.Count; k++)
                {
                    var zobj = (JObj)zarr[k];
                    zobj.Add("produced_energy_cumulative", k < _zoneProducedEnergy.Count ? (object)_zoneProducedEnergy[k] : null);
                    zobj.Add("removed_energy_cumulative", k < _zoneRemovedEnergy.Count ? (object)_zoneRemovedEnergy[k] : null);
                    zobj.Add("recycled_energy_cumulative", null);
                    zobj.Add("decayed_energy_cumulative", null);
                    zobj.Add("counters_evidence", "polling_census_proxy");
                    zobj.Add("produced_energy_exact", FeedingPatches.ZoneValue(FeedingPatches.ZoneProduced, k));
                    zobj.Add("consumed_energy_exact", FeedingPatches.ZoneValue(FeedingPatches.ZoneConsumed, k));
                    zobj.Add("recycled_energy_exact", FeedingPatches.ZoneValue(FeedingPatches.ZoneRecycled, k));
                    zobj.Add("decayed_energy_exact", FeedingPatches.ZoneValue(FeedingPatches.ZoneDecayed, k));
                    zobj.Add("culled_energy_exact", FeedingPatches.ZoneValue(FeedingPatches.ZoneCulled, k));
                    zobj.Add("counters_evidence_exact", "harmony_exact");
                }
                root.Add("zones", zarr);
                var mpOut = new JObj();
                foreach (var kv in matProps) mpOut.Add(kv.Key, kv.Value);
                root.Add("material_properties", mpOut);
                root.Add("totals", new JObj().Add("pellets_n", pelletCount)
                    .Add("plant_amount", plantAmount).Add("meat_amount", meatAmount));
            }
            catch (Exception e)
            {
                root.Add("error", e.GetType().Name + ": " + e.Message);
            }
            return root;
        }

        private static int arrLength(Array a) { return a == null ? 0 : a.Length; }

        private static void AddMaterialProps(Dictionary<string, JObj> props, object mat, string name)
        {
            if (mat == null || name == "unknown" || props.ContainsKey(name)) return;
            props[name] = new JObj()
                .Add("energy_density", F(Inst(mat, "EnergyDensity")))
                .Add("mass_density", F(Inst(mat, "MassDensity")))
                .Add("amount_at_scale1", 20f);
        }

        private static string MaterialName(object mat)
        {
            object name = PropVal(mat, "Name");
            if (name == null) name = PropVal(mat, "name");
            if (name == null) name = Inst(mat, "Name");
            string s = name as string;
            return string.IsNullOrEmpty(s) ? "unknown" : s.ToLowerInvariant();
        }

        private static JObj BodySpriter(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                int bodyId = req.ContainsKey("id") ? Convert.ToInt32(req["id"], CultureInfo.InvariantCulture) : 0;
                var bodyType = FindType("SimulationScripts.BibiteScripts.BibiteBody");
                Component body = null;
                if (bodyType != null)
                    foreach (var obj in UnityEngine.Object.FindObjectsOfType(bodyType))
                    {
                        var b = obj as Component;
                        if (b == null) continue;
                        if (I(Inst(Inst(b, "id"), "id")) == bodyId) { body = b; break; }
                    }
                if (body == null) { o.Add("ok", false).Add("error", "body not found"); return o; }
                MonoBehaviour picked = null;
                foreach (var mb in body.GetComponentsInChildren<MonoBehaviour>(true))
                {
                    if (mb == null) continue;
                    if (mb.GetType().Name.Contains("Procedural") || mb.GetType().Name.Contains("Spriter")) { picked = mb; break; }
                }
                if (picked == null) { o.Add("ok", false).Add("error", "spriter not found"); return o; }
                o.Add("ok", true).Add("type", picked.GetType().Name).Add("path", UiPath(picked.transform));
                var fields = new JObj();
                foreach (var f in picked.GetType().GetFields(BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic))
                {
                    try
                    {
                        var v = f.GetValue(picked);
                        if (v is SpriteRenderer sr)
                            fields.Add(f.Name, new JObj().Add("kind", "SpriteRenderer").Add("path", UiPath(sr.transform))
                                .Add("material", sr.material == null ? null : sr.material.name)
                                .Add("shader", sr.material == null || sr.material.shader == null ? null : sr.material.shader.name)
                                .Add("sprite", sr.sprite == null ? null : sr.sprite.name));
                        else if (v is Material m) fields.Add(f.Name, new JObj().Add("kind", "Material").Add("name", m.name).Add("shader", m.shader == null ? null : m.shader.name));
                        else fields.Add(f.Name, SimpleValue(v));
                    }
                    catch { }
                }
                o.Add("fields", fields);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj BodyScripts(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                int bodyId = req.ContainsKey("id") ? Convert.ToInt32(req["id"], CultureInfo.InvariantCulture) : 0;
                bool en = !req.ContainsKey("enabled") || MiniJson.Truthy(req["enabled"]);
                var bodyType = FindType("SimulationScripts.BibiteScripts.BibiteBody");
                Component body = null;
                if (bodyType != null)
                    foreach (var obj in UnityEngine.Object.FindObjectsOfType(bodyType))
                    {
                        var b = obj as Component;
                        if (b == null) continue;
                        if (I(Inst(Inst(b, "id"), "id")) == bodyId) { body = b; break; }
                    }
                if (body == null) { o.Add("ok", false).Add("error", "body not found"); return o; }
                int n = 0; var list = new JArr();
                foreach (var mb in body.GetComponentsInChildren<MonoBehaviour>(true))
                {
                    if (mb == null) continue;
                    mb.enabled = en; n++;
                    list.Add(new JObj().Add("type", mb.GetType().Name).Add("path", UiPath(mb.transform)).Add("enabled", mb.enabled));
                }
                o.Add("ok", true).Add("enabled", en).Add("count", n).Add("scripts", list);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj NormalizeUi(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                string rootName = req.ContainsKey("root") ? Convert.ToString(req["root"], CultureInfo.InvariantCulture) : "UI";
                float alpha = req.ContainsKey("alpha") ? Convert.ToSingle(req["alpha"], CultureInfo.InvariantCulture) : 1f;
                GameObject root = null;
                foreach (var go in Resources.FindObjectsOfTypeAll(typeof(GameObject)))
                {
                    var g = go as GameObject;
                    if (g != null && g.scene.IsValid() && g.name == rootName) { root = g; break; }
                }
                if (root == null) { o.Add("ok", false).Add("error", "root not found").Add("root", rootName); return o; }
                root.SetActive(true);
                int n = 0;
                var cgType = FindType("UnityEngine.CanvasGroup");
                if (cgType != null)
                {
                    foreach (var cg in root.GetComponentsInChildren(cgType, true))
                    {
                        var c = cg as Component;
                        if (c == null) continue;
                        cgType.GetProperty("alpha")?.SetValue(c, alpha);
                        cgType.GetProperty("interactable")?.SetValue(c, true);
                        cgType.GetProperty("blocksRaycasts")?.SetValue(c, true);
                        n++;
                    }
                }
                o.Add("ok", true).Add("root", rootName).Add("groups_set", n).Add("alpha", alpha);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        /// <summary>POST /d/invoke {"path": ui path of a GameObject, "type": a component type name (short or full), "method": name,
        /// "args": [numbers / strings / bools], "field": optional: a field or property to read afterwards}. Calls a method of a component of
        /// the game's own scripts (e.g. ZoneGroupHandle.ToggleBody(false), EditableLineGraph.SelectPoint(1)); arguments are converted to the
        /// parameter types (int, float, bool, string, enums by name).</summary>
        private static JObj InvokeOn(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                string path = req.ContainsKey("path") ? Convert.ToString(req["path"], CultureInfo.InvariantCulture) : null;
                string typeName = req.ContainsKey("type") ? Convert.ToString(req["type"], CultureInfo.InvariantCulture) : null;
                string method = req.ContainsKey("method") ? Convert.ToString(req["method"], CultureInfo.InvariantCulture) : null;
                var args = req.ContainsKey("args") ? req["args"] as System.Collections.IList : null;
                if (path == null || typeName == null || method == null) { o.Add("ok", false).Add("error", "path, type and method required"); return o; }
                GameObject found = null;
                foreach (var go in Resources.FindObjectsOfTypeAll(typeof(GameObject)))
                {
                    var g = go as GameObject;
                    if (g == null || !g.scene.IsValid()) continue;
                    if (UiPath(g.transform).EndsWith(path, StringComparison.Ordinal)) { found = g; break; }
                }
                if (found == null) { o.Add("ok", false).Add("error", "gameobject not found").Add("path", path); return o; }
                Component target = null;
                foreach (var c in found.GetComponents<Component>())
                {
                    if (c == null) continue;
                    var t = c.GetType();
                    if (t.Name == typeName || t.FullName == typeName) { target = c; break; }
                }
                if (target == null) { o.Add("ok", false).Add("error", "component not found").Add("type", typeName); return o; }
                int want = args == null ? 0 : args.Count;
                System.Reflection.MethodInfo mi = null;
                for (var t = target.GetType(); t != null && mi == null; t = t.BaseType)
                    foreach (var m in t.GetMethods(BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.DeclaredOnly))
                        if (m.Name == method && m.GetParameters().Length == want) { mi = m; break; }
                if (mi == null)
                {
                    var names = new JArr();
                    for (var t = target.GetType(); t != null; t = t.BaseType)
                        foreach (var m in t.GetMethods(BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.DeclaredOnly))
                            if (m.Name.IndexOf("ormalized", StringComparison.OrdinalIgnoreCase) >= 0 || m.Name.IndexOf(method, StringComparison.OrdinalIgnoreCase) >= 0) names.Add(t.Name + "." + m.Name + "(" + m.GetParameters().Length + ")");
                    o.Add("ok", false).Add("error", "method not found").Add("method", method).Add("type", target.GetType().FullName).Add("candidates", names);
                    return o;
                }
                var ps = mi.GetParameters();
                var values = new object[want];
                for (int i = 0; i < want; i++)
                {
                    var pt = ps[i].ParameterType;
                    var a = args[i];
                    if (pt == typeof(int)) values[i] = Convert.ToInt32(a, CultureInfo.InvariantCulture);
                    else if (pt == typeof(float)) values[i] = Convert.ToSingle(a, CultureInfo.InvariantCulture);
                    else if (pt == typeof(double)) values[i] = Convert.ToDouble(a, CultureInfo.InvariantCulture);
                    else if (pt == typeof(bool)) values[i] = MiniJson.Truthy(a);
                    else if (pt == typeof(string)) values[i] = Convert.ToString(a, CultureInfo.InvariantCulture);
                    else if (pt.IsEnum) values[i] = Enum.Parse(pt, Convert.ToString(a, CultureInfo.InvariantCulture));
                    else values[i] = a;
                }
                var result = mi.Invoke(target, values);
                o.Add("ok", true).Add("path", UiPath(found.transform)).Add("method", method);
                if (result != null && (result is string || result.GetType().IsPrimitive)) o.Add("result", result);
                if (req.ContainsKey("field"))
                {
                    string fname = Convert.ToString(req["field"], CultureInfo.InvariantCulture);
                    object v = Inst(target, fname) ?? Pr(target, fname);
                    if (v != null && (v is string || v.GetType().IsPrimitive)) o.Add("value", v);
                    else if (v != null) o.Add("value", v.ToString());
                }
            }
            catch (Exception e)
            {
                var inner = e is System.Reflection.TargetInvocationException && e.InnerException != null ? e.InnerException : e;
                o.Add("ok", false).Add("error", inner.GetType().Name + ": " + inner.Message);
            }
            return o;
        }

        private static JObj ActivatePath(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                string path = req.ContainsKey("path") ? Convert.ToString(req["path"], CultureInfo.InvariantCulture) : null;
                bool active = !req.ContainsKey("active") || MiniJson.Truthy(req["active"]);
                if (string.IsNullOrEmpty(path)) { o.Add("ok", false).Add("error", "path required"); return o; }
                GameObject found = null;
                foreach (var go in Resources.FindObjectsOfTypeAll(typeof(GameObject)))
                {
                    var g = go as GameObject;
                    if (g == null || !g.scene.IsValid()) continue;
                    if (UiPath(g.transform) == path || g.name == path) { found = g; break; }
                }
                if (found == null) { o.Add("ok", false).Add("error", "gameobject not found").Add("path", path); return o; }
                found.SetActive(active);
                o.Add("ok", true).Add("path", UiPath(found.transform)).Add("active", found.activeSelf)
                 .Add("active_in_hierarchy", found.activeInHierarchy);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj ProceduralInfo(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                int bodyId = req.ContainsKey("id") ? Convert.ToInt32(req["id"], CultureInfo.InvariantCulture) : 0;
                var cType = FindType("SimulationScripts.ProceduralSpriteController");
                if (cType == null) { o.Add("ok", false).Add("error", "ProceduralSpriteController type not found"); return o; }
                Component ctl = null;
                foreach (var obj in Resources.FindObjectsOfTypeAll(cType))
                {
                    var c = obj as Component;
                    if (c == null || !c.gameObject.scene.IsValid()) continue;
                    var body = c.GetComponentInParent(FindType("SimulationScripts.BibiteScripts.BibiteBody")) as Component;
                    if (body == null) continue;
                    if (I(Inst(Inst(body, "id"), "id")) == bodyId) { ctl = c; break; }
                }
                if (ctl == null) { o.Add("ok", false).Add("error", "controller not found"); return o; }
                var img = Inst(ctl, "img") as SpriteRenderer;
                o.Add("ok", true).Add("controller", UiPath(ctl.transform));
                if (img != null)
                {
                    o.Add("img_path", UiPath(img.transform)).Add("img_name", img.name)
                     .Add("img_material", img.material == null ? null : img.material.name)
                     .Add("img_shader", img.material == null || img.material.shader == null ? null : img.material.shader.name);
                    if (img.material != null && img.material.shader != null)
                    {
                        var sh = img.material.shader;
                        int count = 0;
                        try { count = (int)sh.GetType().GetMethod("GetPropertyCount", Type.EmptyTypes).Invoke(sh, null); } catch { }
                        var names = new JArr();
                        for (int i = 0; i < count; i++)
                        {
                            try { var n = (string)sh.GetType().GetMethod("GetPropertyName", new[] { typeof(int) }).Invoke(sh, new object[] { i }); if (!string.IsNullOrEmpty(n)) names.Add(n); } catch { }
                        }
                        o.Add("shader_properties", names);
                    }
                }
                o.Add("segment_point", SimpleValue(Inst(ctl, "segmentPoint")));
                o.Add("tail_point", SimpleValue(Inst(ctl, "tailPoint")));
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        // ---- live Body material/shader property dump + float control (tail bend sweep, task-145) ----
        private static SpriteRenderer FindBodyLayerRenderer(int bodyId, string layerName)
        {
            var bodyType = FindType("SimulationScripts.BibiteScripts.BibiteBody");
            if (bodyType == null) return null;
            foreach (var obj in UnityEngine.Object.FindObjectsOfType(bodyType))
            {
                var b = obj as Component;
                if (b == null) continue;
                if (I(Inst(Inst(b, "id"), "id")) != bodyId) continue;
                foreach (var sr in b.GetComponentsInChildren<SpriteRenderer>(true))
                {
                    if (sr == null) continue;
                    if (layerName == null || sr.gameObject.name == layerName || UiPath(sr.transform).EndsWith(layerName, StringComparison.Ordinal))
                        return sr;
                }
            }
            return null;
        }

        // Opens the lineage panel on a species (SpeciesPanel.OpenAndSelectSpecies); req: "id" (speciesID) or "index" (position in recordedSpecies); without either it lists the species.
        private static JObj OpenSpecies(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                var mgrType = FindType("ManagementScripts.GlobalLineageManager");
                var mgr = mgrType.GetField("Instance", BindingFlags.Public | BindingFlags.Static).GetValue(null);
                var list = mgrType.GetField("recordedSpecies", BindingFlags.Public | BindingFlags.Instance).GetValue(mgr) as IList;
                var speciesType = FindType("SimulationScripts.BibiteScripts.Species");
                var idField = speciesType.GetField("speciesID", BindingFlags.Public | BindingFlags.Instance);
                var nameProp = speciesType.GetProperty("name", BindingFlags.Public | BindingFlags.Instance);
                var names = new JArr();
                object chosen = null;
                for (int i = 0; i < list.Count; i++)
                {
                    long sid = Convert.ToInt64(idField.GetValue(list[i]), CultureInfo.InvariantCulture);
                    names.Add(new JObj().Add("index", i).Add("id", sid).Add("name", Convert.ToString(nameProp.GetValue(list[i], null), CultureInfo.InvariantCulture)));
                    if (req.ContainsKey("id") && Convert.ToInt64(req["id"], CultureInfo.InvariantCulture) == sid) chosen = list[i];
                    if (req.ContainsKey("index") && Convert.ToInt32(req["index"], CultureInfo.InvariantCulture) == i) chosen = list[i];
                }
                o.Add("ok", true).Add("count", list.Count).Add("species", names);
                if (chosen != null)
                {
                    var panelType = FindType("UIScripts.UIReferences.LineagePanel.SpeciesPanel");
                    var panel = panelType.GetField("instance", BindingFlags.Public | BindingFlags.Static).GetValue(null);
                    panelType.GetMethod("OpenAndSelectSpecies", BindingFlags.Public | BindingFlags.Instance).Invoke(panel, new[] { chosen });
                    o.Add("opened", true);
                    if (req.ContainsKey("tab"))
                    {
                        // SpeciesTabManager.tabButtons[tab].Select(): the Info (0), Genes (1), Brain (2) or Position (3) tab.
                        int tab = Convert.ToInt32(req["tab"], CultureInfo.InvariantCulture);
                        var mgrT = FindType("UIScripts.SpeciesTabManager");
                        object tabMgr = null;
                        foreach (var candidate in Resources.FindObjectsOfTypeAll(mgrT))
                        {
                            var cc = candidate as Component;
                            if (cc != null && cc.gameObject.scene.IsValid()) { tabMgr = candidate; break; }
                        }
                        var buttons = FindType("UIScripts.TabsManager").GetField("tabButtons", BindingFlags.NonPublic | BindingFlags.Instance).GetValue(tabMgr) as IList;
                        var button = buttons[tab];
                        button.GetType().GetMethod("Select", BindingFlags.Public | BindingFlags.Instance).Invoke(button, null);
                        o.Add("tab", tab);
                    }
                }
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + (e.InnerException != null ? e.InnerException.Message : e.Message)); }
            return o;
        }

        // An int[] field of a UI panel singleton (BiologyPanel.internsMap), row by row as the game keeps it (index = x + y * width).
        private static JObj PanelInts(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                string typeName = req.ContainsKey("type") ? Convert.ToString(req["type"], CultureInfo.InvariantCulture) : "UIScripts.UIPanels.BiologyPanel";
                string field = req.ContainsKey("field") ? Convert.ToString(req["field"], CultureInfo.InvariantCulture) : "internsMap";
                var t = FindType(typeName);
                var inst = t.GetField("instance", BindingFlags.Public | BindingFlags.Static | BindingFlags.NonPublic).GetValue(null);
                var arr = t.GetField(field, BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance).GetValue(inst) as int[];
                if (arr == null) { o.Add("ok", false).Add("error", "no int[]"); return o; }
                var sb = new StringBuilder();
                for (int i = 0; i < arr.Length; i++) sb.Append((char)('0' + Math.Max(0, Math.Min(9, arr[i]))));
                o.Add("ok", true).Add("length", arr.Length).Add("map", sb.ToString());
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        // The pixel program of a UI panel's material, as the GPU runs it: a copy of the material (BiologyPanel.sections_mat / interns by default) with the
        // given float properties set, blitted into a render texture of `w` x `h` exactly as GenerateInternalsMap does, and the red channel of every pixel
        // returned as bytes (row 0 at the bottom). The reference the translated shaders are compared against.
        private static JObj BlitMaterial(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                string typeName = req.ContainsKey("type") ? Convert.ToString(req["type"], CultureInfo.InvariantCulture) : "UIScripts.UIPanels.BiologyPanel";
                string field = req.ContainsKey("field") ? Convert.ToString(req["field"], CultureInfo.InvariantCulture) : "sections_mat";
                int w = req.ContainsKey("w") ? Convert.ToInt32(req["w"], CultureInfo.InvariantCulture) : 61;
                int h = req.ContainsKey("h") ? Convert.ToInt32(req["h"], CultureInfo.InvariantCulture) : 39;
                var t = FindType(typeName);
                var inst = t.GetField("instance", BindingFlags.Public | BindingFlags.Static | BindingFlags.NonPublic).GetValue(null);
                var fi = t.GetField(field, BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance);
                if (fi == null) { o.Add("ok", false).Add("error", "field not found"); return o; }
                var holder = fi.GetValue(inst);
                Material source = holder as Material;
                if (source == null && holder != null)
                {
                    var pm = holder.GetType().GetProperty("material");
                    if (pm != null) source = pm.GetValue(holder, null) as Material;
                }
                if (source == null) { o.Add("ok", false).Add("error", "no material"); return o; }
                var mat = new Material(source);
                // Every request key that starts with an underscore is a material property to set (MiniJson has no nested objects).
                // `_Name.r|g|b|a` sets one component of a colour property; an Int property (`_Width`) is set as an integer (SetFloat does not reach it).
                foreach (var kv in req)
                {
                    if (!kv.Key.StartsWith("_") || !(kv.Value is double)) continue;
                    float val = Convert.ToSingle(kv.Value, CultureInfo.InvariantCulture);
                    int dot = kv.Key.IndexOf('.');
                    if (dot > 0)
                    {
                        string cname = kv.Key.Substring(0, dot);
                        var col = mat.GetColor(cname);
                        switch (kv.Key[dot + 1]) { case 'r': col.r = val; break; case 'g': col.g = val; break; case 'b': col.b = val; break; default: col.a = val; break; }
                        mat.SetColor(cname, col);
                        continue;
                    }
                    int pi = mat.shader.FindPropertyIndex(kv.Key);
                    if (pi >= 0 && mat.shader.GetPropertyType(pi) == UnityEngine.Rendering.ShaderPropertyType.Int) mat.SetInteger(kv.Key, (int)Math.Round(val));
                    else mat.SetFloat(kv.Key, val);
                }
                var rt = RenderTexture.GetTemporary(w, h);
                // A pooled render texture keeps what the last user drew, and the UI programs blend (a transparent pixel leaves the target as it was):
                // clear it first, or the picture is the union of the earlier ones.
                var previous = RenderTexture.active;
                RenderTexture.active = rt;
                GL.Clear(true, true, new Color(0f, 0f, 0f, 0f));
                Graphics.Blit((Texture)null, rt, mat);
                RenderTexture.active = rt;
                var tex = new Texture2D(w, h);
                tex.ReadPixels(new Rect(0f, 0f, (float)w, (float)h), 0, 0);
                tex.Apply();
                RenderTexture.active = previous;
                RenderTexture.ReleaseTemporary(rt);
                var pixels = tex.GetPixels32();
                var reds = new StringBuilder(); var rgba = new StringBuilder();
                foreach (var px in pixels) { reds.Append(px.r.ToString("x2")); rgba.Append(px.r.ToString("x2")).Append(px.g.ToString("x2")).Append(px.b.ToString("x2")).Append(px.a.ToString("x2")); }
                UnityEngine.Object.Destroy(tex); UnityEngine.Object.Destroy(mat);
                o.Add("ok", true).Add("w", w).Add("h", h).Add("shader", source.shader == null ? null : source.shader.name).Add("red_hex", reds.ToString());
                if (req.ContainsKey("rgba")) o.Add("rgba_hex", rgba.ToString());
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + (e.InnerException != null ? e.InnerException.Message : e.Message)); }
            return o;
        }

        // The float properties of a material held by a field of a UI panel singleton (e.g. BiologyPanel.interns / sections_mat), as the game has them now.
        private static JObj PanelMaterial(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                string typeName = req.ContainsKey("type") ? Convert.ToString(req["type"], CultureInfo.InvariantCulture) : "UIScripts.UIPanels.BiologyPanel";
                string field = req.ContainsKey("field") ? Convert.ToString(req["field"], CultureInfo.InvariantCulture) : "interns";
                var t = FindType(typeName);
                if (t == null) { o.Add("ok", false).Add("error", "type not found"); return o; }
                var inst = t.GetField("instance", BindingFlags.Public | BindingFlags.Static | BindingFlags.NonPublic).GetValue(null);
                var fi = t.GetField(field, BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance);
                if (fi == null) { o.Add("ok", false).Add("error", "field not found"); return o; }
                var holder = fi.GetValue(inst);
                Material mat = holder as Material;
                if (mat == null && holder != null)
                {
                    var pm = holder.GetType().GetProperty("material");
                    if (pm != null) mat = pm.GetValue(holder, null) as Material;
                }
                if (mat == null) { o.Add("ok", false).Add("error", "no material"); return o; }
                var shader = mat.shader;
                o.Add("ok", true).Add("material", mat.name).Add("shader", shader == null ? null : shader.name);
                var props = new JArr();
                int count = shader == null ? 0 : shader.GetPropertyCount();
                for (int i = 0; i < count; i++)
                {
                    string pname = shader.GetPropertyName(i);
                    var p = new JObj().Add("name", pname).Add("type", shader.GetPropertyType(i).ToString());
                    try { p.Add("float", mat.GetFloat(pname)); } catch { }
                    try { p.Add("int", mat.GetInteger(pname)); } catch { }
                    if (shader.GetPropertyType(i) == UnityEngine.Rendering.ShaderPropertyType.Color)
                    {
                        var c = mat.GetColor(pname);
                        p.Add("color", new JArr().Add((double)c.r).Add((double)c.g).Add((double)c.b).Add((double)c.a));
                    }
                    props.Add(p);
                }
                o.Add("properties", props);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj MaterialDump(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                int bodyId = req.ContainsKey("id") ? Convert.ToInt32(req["id"], CultureInfo.InvariantCulture) : 0;
                string layer = req.ContainsKey("layer") ? Convert.ToString(req["layer"], CultureInfo.InvariantCulture) : "Body";
                var sr = FindBodyLayerRenderer(bodyId, layer);
                if (sr == null || sr.material == null) { o.Add("ok", false).Add("error", "renderer/material not found"); return o; }
                var mat = sr.material;
                var shader = mat.shader;
                o.Add("ok", true).Add("id", bodyId).Add("layer", layer).Add("path", UiPath(sr.transform))
                 .Add("material", mat.name).Add("shader", shader == null ? null : shader.name);
                if (shader != null)
                {
                    var sh = shader;
                    int count = 0;
                    try { count = (int)sh.GetType().GetMethod("GetPropertyCount", Type.EmptyTypes).Invoke(sh, null); } catch { }
                    var props = new JArr();
                    for (int i = 0; i < count; i++)
                    {
                        string pname = null;
                        try { pname = (string)sh.GetType().GetMethod("GetPropertyName", new[] { typeof(int) }).Invoke(sh, new object[] { i }); } catch { }
                        if (string.IsNullOrEmpty(pname)) continue;
                        object ptype = null;
                        try { ptype = sh.GetType().GetMethod("GetPropertyType", new[] { typeof(int) }).Invoke(sh, new object[] { i }); } catch { }
                        var p = new JObj().Add("index", i).Add("name", pname).Add("type", ptype == null ? null : ptype.ToString())
                            .Add("property_id", Shader.PropertyToID(pname));
                        try { p.Add("float", mat.GetFloat(pname)); } catch { }
                        try { p.Add("color", SimpleValue(mat.GetColor(pname))); } catch { }
                        try { var tx = mat.GetTexture(pname); if (tx != null) p.Add("texture", tx.name); } catch { }
                        props.Add(p);
                    }
                    o.Add("properties", props).Add("property_count", count);
                }
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj MaterialFloat(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                int bodyId = req.ContainsKey("id") ? Convert.ToInt32(req["id"], CultureInfo.InvariantCulture) : 0;
                string layer = req.ContainsKey("layer") ? Convert.ToString(req["layer"], CultureInfo.InvariantCulture) : "Body";
                string name = req.ContainsKey("name") ? Convert.ToString(req["name"], CultureInfo.InvariantCulture) : null;
                float value = req.ContainsKey("value") ? Convert.ToSingle(req["value"], CultureInfo.InvariantCulture) : 0f;
                var sr = FindBodyLayerRenderer(bodyId, layer);
                if (sr == null || sr.material == null || string.IsNullOrEmpty(name))
                { o.Add("ok", false).Add("error", "renderer/material/name not found"); return o; }
                sr.material.SetFloat(name, value);
                try { sr.SetPropertyBlock(null); } catch { }
                o.Add("ok", true).Add("name", name).Add("value", value).Add("material", sr.material.name);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        // ---- exact live Sprite/Texture dump for one body layer (task-144) --------------------------
        private static JObj SpriteDump(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                int bodyId = req.ContainsKey("id") ? Convert.ToInt32(req["id"], CultureInfo.InvariantCulture) : 0;
                string layerName = req.ContainsKey("layer") ? Convert.ToString(req["layer"], CultureInfo.InvariantCulture) : null;
                var bodyType = FindType("SimulationScripts.BibiteScripts.BibiteBody");
                Component body = null;
                if (bodyType != null)
                    foreach (var obj in UnityEngine.Object.FindObjectsOfType(bodyType))
                    {
                        var b = obj as Component;
                        if (b == null) continue;
                        if (I(Inst(Inst(b, "id"), "id")) == bodyId) { body = b; break; }
                    }
                if (body == null) { o.Add("ok", false).Add("error", "body not found"); return o; }
                SpriteRenderer picked = null;
                foreach (var sr in body.GetComponentsInChildren<SpriteRenderer>(true))
                {
                    if (sr == null) continue;
                    if (layerName == null || sr.gameObject.name == layerName || UiPath(sr.transform).EndsWith(layerName, StringComparison.Ordinal))
                    { picked = sr; break; }
                }
                if (picked == null) { o.Add("ok", false).Add("error", "layer not found"); return o; }
                var sprite = picked.sprite;
                if (sprite == null) { o.Add("ok", false).Add("error", "sprite null"); return o; }
                var tex = sprite.texture;
                o.Add("ok", true).Add("body_id", bodyId).Add("layer", picked.gameObject.name)
                 .Add("path", UiPath(picked.transform)).Add("sprite", sprite.name)
                 .Add("texture", tex == null ? null : tex.name)
                 .Add("texture_width", tex == null ? 0 : tex.width).Add("texture_height", tex == null ? 0 : tex.height)
                 .Add("texture_format", tex == null ? null : tex.format.ToString());
                var r = sprite.rect;
                o.Add("sprite_rect", new JObj().Add("x", r.x).Add("y", r.y).Add("w", r.width).Add("h", r.height));
                o.Add("sprite_pivot", new JObj().Add("x", sprite.pivot.x).Add("y", sprite.pivot.y));
                o.Add("pixels_per_unit", sprite.pixelsPerUnit);
                o.Add("rect_texture_rect", new JObj().Add("x", r.x).Add("y", r.y).Add("w", r.width).Add("h", r.height));
                if (tex != null)
                {
                    var encType = FindType("UnityEngine.ImageConversion");
                    var enc = encType == null ? null : encType.GetMethod("EncodeToPNG", new[] { typeof(Texture2D) });
                    if (enc != null)
                    {
                        var png = (byte[])enc.Invoke(null, new object[] { tex });
                        o.Add("texture_png_base64", Convert.ToBase64String(png));
                    }
                }
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        // ---- per-layer SpriteRenderer toggling for visible-alpha closeups (task-142) ------------------
        private static readonly Dictionary<int, KeyValuePair<bool, bool>> RendererState =
            new Dictionary<int, KeyValuePair<bool, bool>>();

        private static object Pr(object instance, string name)
        {
            try { return instance == null ? null : instance.GetType().GetProperty(name, BindingFlags.Public | BindingFlags.Instance).GetValue(instance, null); }
            catch { return null; }
        }

        // Every object that has a component of type `type` (e.g. "SimulationScripts.ColorKiller"), with its whole hierarchy of sprite renderers, UI images and animators:
        // sprite names, colours, materials and shaders, the local transform and the animation clips. For reading how a world object is built.
        private static JObj ObjectRenderers(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                string typeName = Convert.ToString(req["type"], CultureInfo.InvariantCulture);
                var t = FindType(typeName);
                if (t == null) { o.Add("ok", false).Add("error", "type not found"); return o; }
                var list = new JArr();
                foreach (var obj in UnityEngine.Object.FindObjectsOfType(t))
                {
                    var comp = obj as Component;
                    if (comp == null) continue;
                    var nodes = new JArr();
                    foreach (var tr in comp.GetComponentsInChildren<Transform>(true))
                    {
                        var n = new JObj().Add("path", UiPath(tr)).Add("active", tr.gameObject.activeSelf)
                            .Add("local_position", SimpleValue(tr.localPosition)).Add("local_euler_z", tr.localEulerAngles.z)
                            .Add("local_scale", SimpleValue(tr.localScale));
                        var kinds = new JArr();
                        var fieldsObj = new JObj();
                        foreach (var c in tr.GetComponents<Component>())
                        {
                            if (c == null) continue;
                            kinds.Add(c.GetType().FullName);
                            // The plain numbers of the game's own scripts (not Unity's): rates, sizes, flags.
                            if (c.GetType().Namespace != null && c.GetType().Namespace.StartsWith("UnityEngine")) continue;
                            foreach (var f in c.GetType().GetFields(BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance))
                            {
                                if (f.FieldType == typeof(float) || f.FieldType == typeof(int) || f.FieldType == typeof(bool))
                                    fieldsObj.Add(c.GetType().Name + "." + f.Name, Convert.ToString(f.GetValue(c), CultureInfo.InvariantCulture));
                            }
                        }
                        n.Add("components", kinds);
                        n.Add("fields", fieldsObj);
                        var sr = tr.GetComponent<SpriteRenderer>();
                        if (sr != null)
                            n.Add("sprite_renderer", new JObj().Add("sprite", sr.sprite == null ? null : sr.sprite.name).Add("color", SimpleValue(sr.color))
                                .Add("material", sr.sharedMaterial == null ? null : sr.sharedMaterial.name).Add("shader", sr.sharedMaterial == null || sr.sharedMaterial.shader == null ? null : sr.sharedMaterial.shader.name)
                                .Add("sorting_layer", sr.sortingLayerName).Add("sorting_order", sr.sortingOrder).Add("flip_x", sr.flipX).Add("flip_y", sr.flipY)
                                .Add("draw_mode", sr.drawMode.ToString()).Add("size", SimpleValue(sr.size)));
                        var imgType = FindType("UnityEngine.UI.Image");
                        Component img = imgType == null ? null : tr.GetComponent(imgType);
                        if (img != null)
                        {
                            var isprite = Pr(img, "sprite") as Sprite; var imat = Pr(img, "material") as Material;
                            n.Add("image", new JObj().Add("sprite", isprite == null ? null : isprite.name).Add("color", SimpleValue(Pr(img, "color")))
                                .Add("material", imat == null ? null : imat.name).Add("shader", imat == null || imat.shader == null ? null : imat.shader.name));
                        }
                        var animType = FindType("UnityEngine.Animator");
                        Component anim = animType == null ? null : tr.GetComponent(animType);
                        if (anim != null)
                        {
                            var clips = new JArr();
                            var ctrl = Pr(anim, "runtimeAnimatorController");
                            var clipList = ctrl == null ? null : Pr(ctrl, "animationClips") as System.Collections.IEnumerable;
                            if (clipList != null)
                                foreach (var c in clipList)
                                    clips.Add(new JObj().Add("name", Convert.ToString(Pr(c, "name"), CultureInfo.InvariantCulture)).Add("length", Convert.ToDouble(Pr(c, "length"), CultureInfo.InvariantCulture)).Add("loop", Convert.ToBoolean(Pr(c, "isLooping"))));
                            n.Add("animator", new JObj().Add("controller", ctrl == null ? null : Convert.ToString(Pr(ctrl, "name"), CultureInfo.InvariantCulture)).Add("speed", Convert.ToDouble(Pr(anim, "speed"), CultureInfo.InvariantCulture)).Add("clips", clips));
                        }
                        nodes.Add(n);
                    }
                    list.Add(new JObj().Add("path", UiPath(comp.transform)).Add("nodes", nodes));
                }
                o.Add("ok", true).Add("objects", list);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj RendererControl(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                string action = req.ContainsKey("action") ? Convert.ToString(req["action"], CultureInfo.InvariantCulture) : "off";
                string only = req.ContainsKey("path") ? Convert.ToString(req["path"], CultureInfo.InvariantCulture) : null;
                int bodyId = req.ContainsKey("id") ? Convert.ToInt32(req["id"], CultureInfo.InvariantCulture) : 0;
                var bodyType = FindType("SimulationScripts.BibiteScripts.BibiteBody");
                if (bodyType == null) { o.Add("ok", false).Add("error", "body type not found"); return o; }
                Component body = null;
                foreach (var obj in UnityEngine.Object.FindObjectsOfType(bodyType))
                {
                    var b = obj as Component;
                    if (b == null) continue;
                    int id = I(Inst(Inst(b, "id"), "id"));
                    if (id == bodyId) { body = b; break; }
                }
                if (body == null) { o.Add("ok", false).Add("error", "body not found").Add("id", bodyId); return o; }
                var renderers = body.GetComponentsInChildren<SpriteRenderer>(true);
                var arr = new JArr();
                foreach (var sr in renderers)
                {
                    if (sr == null) continue;
                    int key = sr.GetInstanceID();
                    bool wasEnabled = false; bool wasActive = true;
                    if (action == "restore")
                    {
                        KeyValuePair<bool, bool> state;
                        if (RendererState.TryGetValue(key, out state))
                        {
                            sr.enabled = state.Key;
                            if (sr.gameObject.activeSelf != state.Value) sr.gameObject.SetActive(state.Value);
                            RendererState.Remove(key);
                        }
                        arr.Add(new JObj().Add("path", UiPath(sr.transform)).Add("enabled", sr.enabled)
                            .Add("active", sr.gameObject.activeSelf));
                        continue;
                    }
                    if (!RendererState.ContainsKey(key))
                    {
                        wasEnabled = sr.enabled;
                        wasActive = sr.gameObject.activeSelf;
                        RendererState[key] = new KeyValuePair<bool, bool>(wasEnabled, wasActive);
                    }
                    bool target = only == null || UiPath(sr.transform).EndsWith(only, StringComparison.Ordinal);
                    if (action == "only") target = only != null && UiPath(sr.transform).EndsWith(only, StringComparison.Ordinal);
                    if (action == "off") target = false;
                    sr.enabled = target;
                    if (target && !sr.gameObject.activeSelf) sr.gameObject.SetActive(true);
                    arr.Add(new JObj().Add("path", UiPath(sr.transform)).Add("enabled", target)
                        .Add("active", sr.gameObject.activeSelf).Add("name", sr.gameObject.name));
                }
                o.Add("ok", true).Add("action", action).Add("id", bodyId).Add("renderers", arr)
                 .Add("renderer_count", arr.Count);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        // ---- per-SpriteRenderer layer dump for bibite reference closeups (task-135) ------------------
        private static object RP(object o, string name)
        {
            try
            {
                if (o == null) return null;
                var t = o.GetType();
                var p = t.GetProperty(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                if (p != null && p.CanRead) return p.GetValue(o);
                var f = t.GetField(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                return f == null ? null : f.GetValue(o);
            }
            catch { return null; }
        }

        private static object JVal(JObj o, string key)
        {
            if (o == null) return null;
            foreach (var kv in o) if (kv.Key == key) return kv.Value;
            return null;
        }

        private static JObj BibiteLayers()
        {
            var root = new JObj();
            root.Add("schema", "rustibites.assets.bibite_layers.v1");
            root.Add("game", Application.version);
            root.Add("unity", Application.unityVersion);
            var bodyType = FindType("SimulationScripts.BibiteScripts.BibiteBody");
            var srType = FindType("UnityEngine.SpriteRenderer");
            var canvasType = FindType("UnityEngine.Canvas");
            if (bodyType == null || srType == null)
            { root.Add("error", "body/sprite renderer type not found"); return root; }
            var map = new Dictionary<int, JObj>();
            var order = new List<int>();
            try
            {
                foreach (var obj in Resources.FindObjectsOfTypeAll(srType))
                {
                    var sr = obj as Component;
                    if (sr == null || !sr.gameObject.scene.IsValid() || sr.hideFlags != HideFlags.None) continue;
                    var body = sr.GetComponentInParent(bodyType) as Component;
                    if (body == null) continue;
                    int id = I(Inst(Inst(body, "id"), "id"));
                    if (!map.ContainsKey(id))
                    {
                        var tr = body.transform;
                        map[id] = new JObj().Add("id", id).Add("body_name", body.name)
                            .Add("body_position", SimpleValue(tr.position))
                            .Add("body_rotation", SimpleValue(tr.eulerAngles))
                            .Add("body_scale", SimpleValue(tr.lossyScale))
                            .Add("layers", new JArr());
                        order.Add(id);
                    }
                    var t = sr.transform;
                    var o = new JObj().Add("path", UiPath(t)).Add("name", sr.gameObject.name)
                        .Add("sorting_order", RP(sr, "sortingOrder"))
                        .Add("sorting_layer", RP(sr, "sortingLayerName"))
                        .Add("enabled", RP(sr, "enabled")).Add("color", SimpleValue(RP(sr, "color")))
                        .Add("flip_x", RP(sr, "flipX")).Add("flip_y", RP(sr, "flipY"))
                        .Add("draw_mode", RP(sr, "drawMode"))
                        .Add("world_position", SimpleValue(t.position)).Add("world_rotation", SimpleValue(t.eulerAngles))
                        .Add("lossy_scale", SimpleValue(t.lossyScale))
                        .Add("local_position", SimpleValue(t.localPosition)).Add("local_rotation", SimpleValue(t.localEulerAngles))
                        .Add("local_scale", SimpleValue(t.localScale));
                    var sprite = RP(sr, "sprite");
                    var bounds = RP(sr, "bounds");
                    if (bounds != null)
                    {
                        o.Add("bounds_center", SimpleValue(RP(bounds, "center")));
                        o.Add("bounds_size", SimpleValue(RP(bounds, "size")));
                    }
                    if (sprite != null)
                    {
                        o.Add("sprite", RP(sprite, "name"));
                        var rect = RP(sprite, "rect");
                        if (rect != null) { o.Add("sprite_rect", SimpleValue(rect)); }
                        o.Add("sprite_pivot", SimpleValue(RP(sprite, "pivot")));
                        o.Add("pixels_per_unit", RP(sprite, "pixelsPerUnit"));
                        try
                        {
                            var tex = RP(sprite, "texture");
                            o.Add("sprite_texture", tex == null ? null : RP(tex, "name"));
                            o.Add("sprite_texture_size", tex == null ? null : new JObj().Add("w", RP(tex, "width")).Add("h", RP(tex, "height")));
                            if (tex != null && rect != null)
                            {
                                var rr = (Rect)rect;
                                var texType = tex.GetType();
                                MethodInfo gp = texType.GetMethod("GetPixels32", Type.EmptyTypes);
                                var px = gp == null ? null : gp.Invoke(tex, null) as Array;
                                if (px != null)
                                {
                                    int tw = Convert.ToInt32(RP(tex, "width"), CultureInfo.InvariantCulture);
                                    int th = Convert.ToInt32(RP(tex, "height"), CultureInfo.InvariantCulture);
                                    int x0 = (int)rr.x, y0 = (int)rr.y, rw = (int)rr.width, rh = (int)rr.height;
                                    int non = 0;
                                    for (int yy = y0; yy < y0 + rh; yy++)
                                        for (int xx = x0; xx < x0 + rw; xx++)
                                        {
                                            int idx = yy * tw + xx;
                                            if (idx < 0 || idx >= px.Length) continue;
                                            try
                                            {
                                                var c = px.GetValue(idx);
                                                var a = (byte)c.GetType().GetField("a").GetValue(c);
                                                if (a != 0) non++;
                                            }
                                            catch { }
                                        }
                                    o.Add("sprite_region_nontransparent", non);
                                    o.Add("sprite_region_pixels", rw * rh);
                                }
                            }
                        }
                        catch { }
                        // sprite-local corners (pivot origin) -> world, assuming default quad draw mode
                        try
                        {
                            var r = (Rect)rect; var pivot = (Vector2)RP(sprite, "pivot"); float ppu = Convert.ToSingle(RP(sprite, "pixelsPerUnit"), CultureInfo.InvariantCulture);
                            float w = r.width / ppu, h = r.height / ppu;
                            float x0 = -pivot.x * w, y0 = -pivot.y * h;
                            var corners = new JArr();
                            foreach (var c in new[] { new Vector3(x0, y0, 0), new Vector3(x0 + w, y0, 0), new Vector3(x0, y0 + h, 0), new Vector3(x0 + w, y0 + h, 0) })
                                corners.Add(SimpleValue(t.TransformPoint(c)));
                            o.Add("world_corners", corners);
                        }
                        catch { }
                    }
                    ((JArr)JVal(map[id], "layers")).Add(o);
                }
                var arr = new JArr();
                foreach (var id in order)
                {
                    var m = map[id];
                    var layers = (JArr)JVal(m, "layers");
                    float minx = float.MaxValue, miny = float.MaxValue, maxx = float.MinValue, maxy = float.MinValue;
                    foreach (var lo in layers)
                    {
                        var jo = lo as JObj; if (jo == null) continue;
                        var bc = (JObj)JVal(jo, "bounds_center"); var bs = (JObj)JVal(jo, "bounds_size");
                        if (bc == null || bs == null) continue;
                        float cx = Convert.ToSingle(JVal(bc, "x"), CultureInfo.InvariantCulture), cy = Convert.ToSingle(JVal(bc, "y"), CultureInfo.InvariantCulture);
                        float sx = Convert.ToSingle(JVal(bs, "x"), CultureInfo.InvariantCulture), sy = Convert.ToSingle(JVal(bs, "y"), CultureInfo.InvariantCulture);
                        minx = Math.Min(minx, cx - sx / 2); maxx = Math.Max(maxx, cx + sx / 2);
                        miny = Math.Min(miny, cy - sy / 2); maxy = Math.Max(maxy, cy + sy / 2);
                    }
                    if (minx < maxx) m.Add("composited_bbox", new JObj().Add("x0", minx).Add("y0", miny).Add("x1", maxx).Add("y1", maxy)
                        .Add("w", maxx - minx).Add("h", maxy - miny).Add("cx", (minx + maxx) / 2).Add("cy", (miny + maxy) / 2));
                    arr.Add(m);
                }
                root.Add("bibites", arr);
                root.Add("composited", true);
            }
            catch (Exception e) { root.Add("error", e.GetType().Name + ": " + e.Message); }
            return root;
        }

        // ---- runtime UI field dump (task-102) -------------------------------------------------------
        // Walks the live Canvas hierarchies (active and inactive) and reads the managed fields that asset
        // files lose because MonoBehaviours have no typetrees: Image sprite/color/type/border/fill, TMP/Text
        // font/size/alignment/color/text, CanvasScaler reference resolution/match/scaleFactor, layout-group
        // paddings/spacing, Selectable state. RectTransform geometry included for the ui_layout.json join.
        private JObj BuildUiRuntime()
        {
            var root = new JObj();
            root.Add("schema", "rustibites.assets.ui_runtime.v1");
            root.Add("plugin", Version);
            root.Add("game", Application.version);
            root.Add("unity", Application.unityVersion);
            root.Add("createdAtUtc", DateTime.UtcNow.ToString("o", CultureInfo.InvariantCulture));
            root.Add("screen", new JObj().Add("width", Screen.width).Add("height", Screen.height)
                .Add("dpi", Screen.dpi).Add("fullscreen", Screen.fullScreen));
            root.Add("cameras", UiCameras());
            var canvases = new JArr();
            var visited = new HashSet<int>();
            try
            {
                var canvasType = FindType("UnityEngine.Canvas");
                if (canvasType == null) { root.Add("error", "UnityEngine.Canvas type not found"); return root; }
                var all = Resources.FindObjectsOfTypeAll(canvasType);
                foreach (var obj in all)
                {
                    var cv = obj as Component;
                    if (cv == null) continue;
                    var go = cv.gameObject;
                    if (!go.scene.IsValid() || go.hideFlags != HideFlags.None) continue;
                    int id = go.GetInstanceID();
                    if (!visited.Add(id)) continue;
                    canvases.Add(UiNode(go, visited));
                }
                root.Add("canvases", canvases);
                root.Add("node_count", visited.Count);
            }
            catch (Exception e)
            {
                root.Add("error", e.GetType().Name + ": " + e.Message);
            }
            return root;
        }

        // ---- HUD reference capture: render only the UICamera's ScreenSpaceCamera canvases to a
        // transparent RenderTexture so the caller can composite them over the main-camera world shot.
        private JObj UiOverlayCapture(int w, int h)
        {
            var o = new JObj();
            var restores = new List<Action>();
            try
            {
                var canvasType = FindType("UnityEngine.Canvas");
                var main = Camera.main;
                if (canvasType == null || main == null)
                { o.Add("ok", false).Add("error", "Canvas/Main Camera not found"); return o; }
                canvasType.GetMethod("ForceUpdateCanvases", BindingFlags.Public | BindingFlags.Static)
                    ?.Invoke(null, null);
                // ScreenSpaceCamera canvases only draw for their worldCamera. Point them at the main
                // camera for this offscreen pass, and give the main camera the UI camera transform/
                // projection so the canvas scaling matches; restore everything afterwards.
                int switched = 0;
                foreach (var obj in Resources.FindObjectsOfTypeAll(canvasType))
                {
                    var cv = obj as Component;
                    if (cv == null || !cv.gameObject.scene.IsValid()) continue;
                    var cvType = cv.GetType();
                    var modeProp = cvType.GetProperty("renderMode");
                    string mode = Convert.ToString(modeProp == null ? null : modeProp.GetValue(cv), CultureInfo.InvariantCulture);
                    if (mode != "ScreenSpaceCamera") continue;
                    var wcProp = cvType.GetProperty("worldCamera");
                    if (wcProp == null) continue;
                    var oldCam = wcProp.GetValue(cv);
                    var cvLocal = cv;
                    switched++;
                    restores.Add(() => { try { wcProp.SetValue(cvLocal, oldCam); } catch { } });
                    wcProp.SetValue(cv, main);
                }
                var ui = FindNamedCamera("UICamera");
                var tr = main.transform;
                var oldPos = tr.position; var oldRot = tr.rotation;
                var oldOrtho = main.orthographic; var oldSize = main.orthographicSize;
                var oldAspect = main.aspect; var oldMask = main.cullingMask;
                var oldClear = main.clearFlags; var oldColor = main.backgroundColor;
                var oldTarget = main.targetTexture; var oldRect = main.rect;
                restores.Add(() => {
                    tr.position = oldPos; tr.rotation = oldRot;
                    main.orthographic = oldOrtho; main.orthographicSize = oldSize;
                    main.aspect = oldAspect; main.cullingMask = oldMask;
                    main.clearFlags = oldClear; main.backgroundColor = oldColor;
                    main.targetTexture = oldTarget; main.rect = oldRect;
                });
                if (ui != null)
                {
                    tr.position = ui.transform.position;
                    tr.rotation = ui.transform.rotation;
                    main.orthographic = ui.orthographic;
                    main.orthographicSize = ui.orthographicSize;
                    main.cullingMask = ui.cullingMask;
                }
                else
                {
                    main.orthographic = true;
                    main.orthographicSize = h / 2f;
                }
                var rt = RenderTexture.GetTemporary(w, h, 24, RenderTextureFormat.ARGB32);
                restores.Add(() => RenderTexture.ReleaseTemporary(rt));
                main.targetTexture = rt;
                main.clearFlags = CameraClearFlags.SolidColor;
                main.backgroundColor = new Color(0f, 0f, 0f, 0f);
                main.aspect = (float)w / h;
                main.rect = new Rect(0f, 0f, 1f, 1f);
                canvasType.GetMethod("ForceUpdateCanvases", BindingFlags.Public | BindingFlags.Static)
                    ?.Invoke(null, null);
                main.Render();
                RenderTexture.active = rt;
                var tex = new Texture2D(w, h, TextureFormat.RGBA32, false);
                tex.ReadPixels(new Rect(0, 0, w, h), 0, 0);
                tex.Apply(false, false);
                RenderTexture.active = null;
                var encType = FindType("UnityEngine.ImageConversion");
                var enc = encType?.GetMethod("EncodeToPNG", new[] { typeof(Texture2D) });
                if (enc == null) throw new InvalidOperationException("ImageConversion.EncodeToPNG not found");
                var png = (byte[])enc.Invoke(null, new object[] { tex });
                int nonZero = 0;
                var px = tex.GetPixels32();
                for (int i = 0; i < px.Length; i++) if (px[i].a != 0) nonZero++;
                UnityEngine.Object.Destroy(tex);
                o.Add("ok", true).Add("camera", main.name).Add("width", w).Add("height", h)
                 .Add("switched_canvases", switched).Add("nonzero_alpha_pixels", nonZero)
                 .Add("png_base64", Convert.ToBase64String(png));
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            finally
            {
                for (int i = restores.Count - 1; i >= 0; i--) { try { restores[i](); } catch { } }
            }
            return o;
        }

        private JObj PressButton(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                string wantName = req.ContainsKey("name") ? Convert.ToString(req["name"], CultureInfo.InvariantCulture) : null;
                string wantPath = req.ContainsKey("path") ? Convert.ToString(req["path"], CultureInfo.InvariantCulture) : null;
                var btnType = FindType("UnityEngine.UI.Button");
                if (btnType == null) { o.Add("ok", false).Add("error", "UI.Button not found"); return o; }
                foreach (var obj in Resources.FindObjectsOfTypeAll(btnType))
                {
                    var b = obj as Component;
                    if (b == null || !b.gameObject.scene.IsValid()) continue;
                    string path = UiPath(b.transform);
                    bool match = (wantPath != null && path.EndsWith(wantPath, StringComparison.Ordinal))
                                 || (wantPath == null && wantName != null &&
                                     string.Equals(b.gameObject.name, wantName, StringComparison.OrdinalIgnoreCase));
                    if (!match) continue;
                    var onClick = btnType.GetProperty("onClick")?.GetValue(b);
                    var invoke = onClick == null ? null : onClick.GetType().GetMethod("Invoke", Type.EmptyTypes);
                    if (invoke == null) { o.Add("ok", false).Add("error", "onClick.Invoke not found"); return o; }
                    invoke.Invoke(onClick, null);
                    o.Add("ok", true).Add("pressed", b.gameObject.name).Add("path", path);
                    return o;
                }
                o.Add("ok", false).Add("error", "button not found").Add("name", wantName).Add("path", wantPath);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj ControlDump()
        {
            var o = new JObj(); var arr = new JArr();
            try
            {
                var toggleType = FindType("UnityEngine.UI.Toggle");
                var sliderType = FindType("UnityEngine.UI.Slider");
                var dropType = FindType("TMPro.TMP_Dropdown");
                var inputType = FindType("TMPro.TMP_InputField");
                var tmpType = FindType("TMPro.TextMeshProUGUI");
                if (toggleType != null)
                    foreach (var obj in UnityEngine.Object.FindObjectsOfType(toggleType))
                    {
                        var c = obj as Component; if (c == null || !c.gameObject.activeInHierarchy) continue;
                        var e = new JObj().Add("kind", "Toggle").Add("path", UiPath(c.transform)).Add("name", c.gameObject.name);
                        try { e.Add("is_on", toggleType.GetProperty("isOn").GetValue(c)); } catch { }
                        try { e.Add("interactable", toggleType.GetProperty("interactable").GetValue(c)); } catch { }
                        AddLabels(e, c, tmpType); arr.Add(e);
                    }
                if (sliderType != null)
                    foreach (var obj in UnityEngine.Object.FindObjectsOfType(sliderType))
                    {
                        var c = obj as Component; if (c == null || !c.gameObject.activeInHierarchy) continue;
                        var e = new JObj().Add("kind", "Slider").Add("path", UiPath(c.transform)).Add("name", c.gameObject.name);
                        foreach (var f in new[] { "value", "minValue", "maxValue", "wholeNumbers", "interactable" })
                            try { e.Add(f, sliderType.GetProperty(f).GetValue(c)); } catch { }
                        AddLabels(e, c, tmpType); arr.Add(e);
                    }
                if (dropType != null)
                    foreach (var obj in UnityEngine.Object.FindObjectsOfType(dropType))
                    {
                        var c = obj as Component; if (c == null || !c.gameObject.activeInHierarchy) continue;
                        var e = new JObj().Add("kind", "Dropdown").Add("path", UiPath(c.transform)).Add("name", c.gameObject.name);
                        try { e.Add("value", dropType.GetProperty("value").GetValue(c)); } catch { }
                        try
                        {
                            var opts = dropType.GetProperty("options").GetValue(c) as System.Collections.IEnumerable;
                            var os = new JArr();
                            if (opts != null) foreach (var op in opts) { var t = op.GetType().GetProperty("text"); os.Add(t == null ? null : t.GetValue(op)); }
                            e.Add("options", os);
                        }
                        catch { }
                        AddLabels(e, c, tmpType); arr.Add(e);
                    }
                if (inputType != null)
                    foreach (var obj in UnityEngine.Object.FindObjectsOfType(inputType))
                    {
                        var c = obj as Component; if (c == null || !c.gameObject.activeInHierarchy) continue;
                        var e = new JObj().Add("kind", "Input").Add("path", UiPath(c.transform)).Add("name", c.gameObject.name);
                        try { e.Add("text", inputType.GetProperty("text").GetValue(c)); } catch { }
                        AddLabels(e, c, tmpType); arr.Add(e);
                    }
                o.Add("ok", true).Add("controls", arr).Add("count", arr.Count);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static void AddLabels(JObj e, Component c, Type tmpType)
        {
            try
            {
                var labels = new JArr();
                foreach (var t in c.GetComponentsInChildren(tmpType, true))
                {
                    var v = tmpType.GetField("text", BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic)?.GetValue(t);
                    if (v != null) labels.Add(Convert.ToString(v, CultureInfo.InvariantCulture));
                }
                e.Add("labels", labels);
            }
            catch { }
        }

        private static JObj ControlSet(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                string path = req.ContainsKey("path") ? Convert.ToString(req["path"], CultureInfo.InvariantCulture) : null;
                string kind = req.ContainsKey("kind") ? Convert.ToString(req["kind"], CultureInfo.InvariantCulture) : null;
                object value = req.ContainsKey("value") ? req["value"] : null;
                var typeName = kind == "Toggle" ? "UnityEngine.UI.Toggle" : kind == "Slider" ? "UnityEngine.UI.Slider" :
                               kind == "Dropdown" ? "TMPro.TMP_Dropdown" : kind == "Input" ? "TMPro.TMP_InputField" : null;
                var t = typeName == null ? null : FindType(typeName);
                if (t == null || string.IsNullOrEmpty(path)) { o.Add("ok", false).Add("error", "kind/path missing"); return o; }
                Component found = null;
                foreach (var obj in Resources.FindObjectsOfTypeAll(t))
                {
                    var c = obj as Component; if (c == null || !c.gameObject.scene.IsValid()) continue;
                    if (UiPath(c.transform) == path) { found = c; break; }
                }
                if (found == null) { o.Add("ok", false).Add("error", "control not found").Add("path", path); return o; }
                if (kind == "Toggle") t.GetProperty("isOn").SetValue(found, MiniJson.Truthy(value));
                else if (kind == "Slider") t.GetProperty("value").SetValue(found, Convert.ToSingle(value, CultureInfo.InvariantCulture));
                else if (kind == "Dropdown") t.GetProperty("value").SetValue(found, Convert.ToInt32(value, CultureInfo.InvariantCulture));
                else if (kind == "Input") t.GetProperty("text").SetValue(found, Convert.ToString(value, CultureInfo.InvariantCulture));
                var evName = kind == "Toggle" ? "onValueChanged" : kind == "Slider" ? "onValueChanged" : kind == "Dropdown" ? "onValueChanged" : "onValueChanged";
                try
                {
                    var ev = t.GetProperty(evName).GetValue(found);
                    var arg = kind == "Toggle" ? (object)MiniJson.Truthy(value) : kind == "Slider" ? Convert.ToSingle(value, CultureInfo.InvariantCulture) : (object)Convert.ToInt32(value, CultureInfo.InvariantCulture);
                    var m = ev.GetType().GetMethod("Invoke", new[] { arg.GetType() });
                    m.Invoke(ev, new[] { arg });
                }
                catch { }
                o.Add("ok", true).Add("path", path).Add("kind", kind);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj PressLabel(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                string want = req.ContainsKey("label") ? Convert.ToString(req["label"], CultureInfo.InvariantCulture) : null;
                bool exact = req.ContainsKey("exact") && MiniJson.Truthy(req["exact"]);
                bool activeOnly = !req.ContainsKey("active") || MiniJson.Truthy(req["active"]);
                var btnType = FindType("UnityEngine.UI.Button");
                var tmpType = FindType("TMPro.TextMeshProUGUI");
                if (btnType == null) { o.Add("ok", false).Add("error", "Button type not found"); return o; }
                var hits = new JArr();
                foreach (var obj in Resources.FindObjectsOfTypeAll(btnType))
                {
                    var b = obj as Component;
                    if (b == null || !b.gameObject.scene.IsValid()) continue;
                    if (activeOnly && !b.gameObject.activeInHierarchy) continue;
                    string label = null;
                    if (tmpType != null)
                        foreach (var t in b.GetComponentsInChildren(tmpType, true))
                        {
                            try { var v = tmpType.GetField("text", BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic)?.GetValue(t); if (v != null) { label = Convert.ToString(v, CultureInfo.InvariantCulture); break; } } catch { }
                        }
                    if (string.IsNullOrEmpty(label)) label = b.gameObject.name;
                    bool match = want == null || (exact ? label == want : label.IndexOf(want, StringComparison.OrdinalIgnoreCase) >= 0);
                    var h = new JObj().Add("path", UiPath(b.transform)).Add("name", b.gameObject.name).Add("label", label);
                    if (!match) continue;
                    try
                    {
                        var onClick = btnType.GetProperty("onClick").GetValue(b);
                        onClick.GetType().GetMethod("Invoke", Type.EmptyTypes).Invoke(onClick, null);
                        h.Add("clicked", true);
                    }
                    catch (Exception ex) { h.Add("clicked", false).Add("error", ex.GetType().Name + ": " + ex.Message); }
                    hits.Add(h);
                    if (hits.Count >= 8) break;
                }
                o.Add("ok", hits.Count > 0).Add("label", want).Add("hits", hits).Add("count", hits.Count);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj SliderSet(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                string path = req.ContainsKey("path") ? Convert.ToString(req["path"], CultureInfo.InvariantCulture) : null;
                float val = req.ContainsKey("value") ? Convert.ToSingle(req["value"], CultureInfo.InvariantCulture) : 0f;
                var sliderType = FindType("UnityEngine.UI.Slider");
                if (sliderType == null || string.IsNullOrEmpty(path)) { o.Add("ok", false).Add("error", "slider/path missing"); return o; }
                Component found = null;
                foreach (var obj in Resources.FindObjectsOfTypeAll(sliderType))
                {
                    var c = obj as Component;
                    if (c == null || !c.gameObject.scene.IsValid()) continue;
                    if (UiPath(c.transform) == path) { found = c; break; }
                }
                if (found == null) { o.Add("ok", false).Add("error", "slider not found").Add("path", path); return o; }
                sliderType.GetProperty("value").SetValue(found, val);
                o.Add("ok", true).Add("path", path).Add("value", sliderType.GetProperty("value").GetValue(found));
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj DigestionBrain()
        {
            var o = new JObj();
            var arr = new JArr();
            try
            {
                var bodyType = FindType("SimulationScripts.BibiteScripts.BibiteBody");
                var stomachType = FindType("SimulationScripts.BibiteScripts.BibiteStomach");
                if (bodyType == null) { o.Add("ok", false).Add("error", "BibiteBody not found"); return o; }
                foreach (var obj in UnityEngine.Object.FindObjectsOfType(bodyType))
                {
                    var body = obj as Component;
                    if (body == null || !body.gameObject.scene.IsValid()) continue;
                    int id = 0; try { id = I(Inst(Inst(body, "id"), "id")); } catch { }
                    var e = new JObj().Add("id", id);
                    try { e.Add("acidLevel", Inst(Inst(body, "stomach"), "acidLevel")); } catch { }
                    try
                    {
                        var brain = Inst(body, "brain");
                        var m = brain.GetType().GetMethod("Output");
                        var p0 = m.GetParameters()[0];
                        var en = Enum.Parse(p0.ParameterType, "Digestion");
                        e.Add("digestionOutput", m.Invoke(brain, new object[] { en }));
                    }
                    catch { }
                    try
                    {
                        var st = Inst(body, "stomach");
                        e.Add("nContent", Inst(st, "nContent")).Add("totalContent", Inst(st, "totalContent"))
                         .Add("digestionEffectiveness", Inst(st, "digestionEffectiveness"))
                         .Add("acidRatio", Inst(st, "acidRatio"));
                    }
                    catch { }
                    arr.Add(e);
                }
                o.Add("ok", true).Add("bibites", arr);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj FovDigest()
        {
            var o = new JObj();
            try
            {
                var fovType = FindType("SimulationScripts.BibiteScripts.FieldOfView");
                var stomachType = FindType("SimulationScripts.BibiteScripts.BibiteStomach");
                var bodyType = FindType("SimulationScripts.BibiteScripts.BibiteBody");
                var fovs = new JArr(); var stomachs = new JArr();
                if (fovType != null)
                    foreach (var obj in UnityEngine.Object.FindObjectsOfType(fovType))
                    {
                        var f = obj as Component;
                        if (f == null || !f.gameObject.scene.IsValid()) continue;
                        int id = 0; Component body = null;
                        try { body = f.GetComponentInParent(bodyType) as Component; if (body != null) id = I(Inst(Inst(body, "id"), "id")); } catch { }
                        var e = new JObj().Add("id", id).Add("needToSee", Inst(f, "needToSee"))
                            .Add("viewRadius", Inst(f, "viewRadius")).Add("viewAngle", Inst(f, "viewAngle"))
                            .Add("nPlants", Inst(f, "nPlants")).Add("nMeat", Inst(f, "nMeat")).Add("nBibite", Inst(f, "nBibite"))
                            .Add("nPlantsInRange", Inst(f, "nPlantsInRange")).Add("nMeatsInRange", Inst(f, "nMeatsInRange"))
                            .Add("nBibitesInRange", Inst(f, "nBibitesInRange")).Add("nCorpsesInRange", Inst(f, "nCorpsesInRange"))
                            .Add("plantWeightSum", Inst(f, "plantWeightSum")).Add("meatWeightSum", Inst(f, "meatWeightSum"))
                            .Add("bibiteWeightSum", Inst(f, "bibiteWeightSum"));
                        foreach (var arrName in new[] { "seenBibites", "seenCorpses", "seenPlantPellets", "seenMeatPellets" })
                        {
                            try
                            {
                                var arr = Inst(f, arrName) as Array;
                                int n = 0; var ids = new JArr();
                                if (arr != null) foreach (var v in arr) { if (v == null) continue; n++; if (n <= 12) ids.Add(v is Component c ? c.GetInstanceID() : 0); }
                                e.Add(arrName + "_n", n).Add(arrName + "_ids", ids);
                            }
                            catch { }
                        }
                        fovs.Add(e);
                    }
                if (stomachType != null)
                    foreach (var obj in UnityEngine.Object.FindObjectsOfType(stomachType))
                    {
                        var st = obj as Component;
                        if (st == null || !st.gameObject.scene.IsValid()) continue;
                        int id = 0; try { var body = st.GetComponentInParent(bodyType) as Component; if (body != null) id = I(Inst(Inst(body, "id"), "id")); } catch { }
                        var e = new JObj().Add("id", id).Add("nContent", Inst(st, "nContent"))
                            .Add("totalContent", Inst(st, "totalContent")).Add("acidLevel", Inst(st, "acidLevel"))
                            .Add("acidRatio", Inst(st, "acidRatio")).Add("digestionEffectiveness", Inst(st, "digestionEffectiveness"))
                            .Add("digestionPotentialBonus", Inst(st, "digestionPotentialBonus")).Add("efficiencyMalus", Inst(st, "efficiencyMalus"))
                            .Add("energyGainRate", Inst(st, "EnergyGainRate"));
                        try
                        {
                            var arr = Inst(st, "stomachContents") as Array;
                            var cs = new JArr(); int n = 0;
                            if (arr != null) foreach (var v in arr)
                            {
                                if (v == null) continue;
                                var boxed = v; var t = boxed.GetType();
                                float amt = Convert.ToSingle(t.GetField("amount").GetValue(boxed), CultureInfo.InvariantCulture);
                                if (amt <= 0.0001f) continue;
                                n++;
                                cs.Add(new JObj().Add("material", Convert.ToString(t.GetField("material").GetValue(boxed), CultureInfo.InvariantCulture))
                                    .Add("amount", amt).Add("digestionRate", Convert.ToSingle(t.GetField("digestionRate").GetValue(boxed), CultureInfo.InvariantCulture))
                                    .Add("lastConversionEfficiency", Convert.ToSingle(t.GetField("lastConversionEfficiency").GetValue(boxed), CultureInfo.InvariantCulture))
                                    .Add("averageChunkAmount", Convert.ToSingle(t.GetField("averageChunkAmount").GetValue(boxed), CultureInfo.InvariantCulture)));
                            }
                            e.Add("contents", cs).Add("contents_n", n);
                        }
                        catch { }
                        stomachs.Add(e);
                    }
                o.Add("ok", true).Add("fov", fovs).Add("stomachs", stomachs).Add("fov_count", fovs.Count).Add("stomach_count", stomachs.Count);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj CameraControls()
        {
            var o = new JObj();
            try
            {
                var cmType = FindType("ManagementScripts.CameraManager");
                if (cmType == null) { o.Add("ok", false).Add("error", "CameraManager type not found"); return o; }
                var instField = cmType.GetField("instance", BindingFlags.Public | BindingFlags.Static);
                var inst = instField == null ? null : instField.GetValue(null);
                o.Add("ok", true).Add("type", cmType.FullName);
                if (inst != null)
                {
                    var fields = new JObj();
                    foreach (var f in cmType.GetFields(BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic))
                    {
                        try
                        {
                            string fn = f.Name;
                            if (fn.Contains("scroll") || fn.Contains("speed") || fn.Contains("zoom") || fn.Contains("camSize")
                                || fn.Contains("start") || fn.Contains("controller") || fn.Contains("Camera"))
                                fields.Add(fn, SimpleValue(f.GetValue(inst)));
                        }
                        catch { }
                    }
                    o.Add("fields", fields);
                    try { o.Add("camSize_static", SimpleValue(cmType.GetField("camSize", BindingFlags.Public | BindingFlags.Static)?.GetValue(null))); } catch { }
                }
                var cam = Camera.main;
                if (cam != null)
                {
                    o.Add("main_camera", new JObj().Add("name", cam.name).Add("orthographic", cam.orthographic)
                        .Add("orthographic_size", cam.orthographicSize).Add("position", SimpleValue(cam.transform.position))
                        .Add("rotation", SimpleValue(cam.transform.eulerAngles)).Add("rect", SimpleValue(cam.rect))
                        .Add("pixel_rect", SimpleValue(cam.pixelRect)).Add("near", cam.nearClipPlane).Add("far", cam.farClipPlane));
                }
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj MaterialProps(Material mat)
        {
            var o = new JObj();
            if (mat == null) return o;
            o.Add("name", mat.name).Add("shader", mat.shader == null ? null : mat.shader.name)
             .Add("render_queue", mat.renderQueue);
            try { var kws = mat.shaderKeywords; if (kws != null) { var ka = new JArr(); foreach (var k in kws) ka.Add(k); o.Add("keywords", ka); } } catch { }
            var sh = mat.shader;
            if (sh == null) return o;
            int count = 0;
            try { count = (int)sh.GetType().GetMethod("GetPropertyCount", Type.EmptyTypes).Invoke(sh, null); } catch { }
            var props = new JArr();
            for (int i = 0; i < count; i++)
            {
                string name = null; object ptype = null;
                try { name = (string)sh.GetType().GetMethod("GetPropertyName", new[] { typeof(int) }).Invoke(sh, new object[] { i }); } catch { }
                try { ptype = sh.GetType().GetMethod("GetPropertyType", new[] { typeof(int) }).Invoke(sh, new object[] { i }); } catch { }
                if (string.IsNullOrEmpty(name)) continue;
                var p = new JObj().Add("index", i).Add("name", name).Add("type", ptype == null ? null : ptype.ToString())
                    .Add("property_id", Shader.PropertyToID(name));
                try { p.Add("float", mat.GetFloat(name)); } catch { }
                try { p.Add("color", SimpleValue(mat.GetColor(name))); } catch { }
                try { var tx = mat.GetTexture(name); if (tx != null) p.Add("texture", tx.name); } catch { }
                props.Add(p);
            }
            o.Add("properties", props).Add("property_count", count);
            return o;
        }

        private static JObj ZoneMaterials()
        {
            var o = new JObj();
            try
            {
                var zoneType = FindType("SimulationScripts.Zone");
                if (zoneType == null) { o.Add("ok", false).Add("error", "Zone type not found"); return o; }
                var zones = new JArr();
                foreach (var obj in UnityEngine.Object.FindObjectsOfType(zoneType))
                {
                    var z = obj as Component;
                    if (z == null || !z.gameObject.scene.IsValid()) continue;
                    var oo = new JObj().Add("zone", z.name);
                    try { oo.Add("id", Inst(z, "id")); } catch { }
                    var sr = Inst(z, "sr") as SpriteRenderer;
                    var live = Inst(z, "mat") as Material;
                    var orig = Inst(z, "zoneMaterial") as Material;
                    oo.Add("sprite_renderer", sr == null ? null : sr.name)
                      .Add("material", live == null ? null : live.name)
                      .Add("zone_material", orig == null ? null : orig.name);
                    if (sr != null) oo.Add("renderer", new JObj().Add("color", SimpleValue(sr.color))
                        .Add("sorting_order", sr.sortingOrder).Add("sorting_layer", sr.sortingLayerName)
                        .Add("enabled", sr.enabled).Add("bounds_center", SimpleValue(sr.bounds.center)).Add("bounds_size", SimpleValue(sr.bounds.size)));
                    oo.Add("live_material_props", MaterialProps(live));
                    oo.Add("source_material_props", MaterialProps(orig));
                    zones.Add(oo);
                }
                o.Add("ok", true).Add("zones", zones).Add("zone_count", zones.Count);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj ScreenCaptureJson()
        {
            var o = new JObj();
            try
            {
                var scType = FindType("UnityEngine.ScreenCapture");
                if (scType == null) { o.Add("ok", false).Add("error", "ScreenCapture type not found"); return o; }
                var m = scType.GetMethod("CaptureScreenshotAsTexture", BindingFlags.Public | BindingFlags.Static, null, Type.EmptyTypes, null);
                var tex = m == null ? null : m.Invoke(null, null) as Texture2D;
                if (tex == null) { o.Add("ok", false).Add("error", "CaptureScreenshotAsTexture returned null"); return o; }
                var encType = FindType("UnityEngine.ImageConversion");
                var enc = encType?.GetMethod("EncodeToPNG", new[] { typeof(Texture2D) });
                var png = enc == null ? null : (byte[])enc.Invoke(null, new object[] { tex });
                UnityEngine.Object.Destroy(tex);
                o.Add("ok", true).Add("width", 0).Add("height", 0).Add("png_base64", Convert.ToBase64String(png));
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private JObj MenuBackground(int w, int h)
        {
            var o = new JObj();
            try
            {
                Camera pick = null;
                foreach (var obj in Resources.FindObjectsOfTypeAll(typeof(Camera)))
                {
                    var cam = obj as Camera;
                    if (cam == null || !cam.gameObject.scene.IsValid() || cam.hideFlags != HideFlags.None) continue;
                    string n = cam.name.ToLowerInvariant();
                    if (n.Contains("ui")) continue;
                    if (pick == null || cam.cullingMask != 0) pick = cam;
                }
                if (pick == null) pick = Camera.main;
                if (pick == null) { o.Add("ok", false).Add("error", "no menu background camera"); return o; }
                var tr = pick.transform; var oldPos = tr.position; var oldRot = tr.rotation;
                var oldTarget = pick.targetTexture; var oldClear = pick.clearFlags; var oldColor = pick.backgroundColor;
                var oldRect = pick.rect; var oldAspect = pick.aspect;
                var rt = RenderTexture.GetTemporary(w, h, 24, RenderTextureFormat.ARGB32);
                try
                {
                    pick.targetTexture = rt; pick.clearFlags = CameraClearFlags.SolidColor;
                    pick.backgroundColor = new Color(0f, 0f, 0f, 1f);
                    pick.aspect = (float)w / h; pick.rect = new Rect(0f, 0f, 1f, 1f);
                    pick.Render();
                    RenderTexture.active = rt;
                    var tex = new Texture2D(w, h, TextureFormat.RGBA32, false);
                    tex.ReadPixels(new Rect(0, 0, w, h), 0, 0); tex.Apply(false, false);
                    RenderTexture.active = null;
                    var encType = FindType("UnityEngine.ImageConversion");
                    var enc = encType?.GetMethod("EncodeToPNG", new[] { typeof(Texture2D) });
                    if (enc == null) throw new InvalidOperationException("EncodeToPNG not found");
                    var png = (byte[])enc.Invoke(null, new object[] { tex });
                    UnityEngine.Object.Destroy(tex);
                    o.Add("ok", true).Add("camera", pick.name).Add("width", w).Add("height", h)
                     .Add("png_base64", Convert.ToBase64String(png));
                }
                finally
                {
                    pick.targetTexture = oldTarget; pick.clearFlags = oldClear; pick.backgroundColor = oldColor;
                    pick.rect = oldRect; pick.aspect = oldAspect; tr.position = oldPos; tr.rotation = oldRot;
                    RenderTexture.active = null; RenderTexture.ReleaseTemporary(rt);
                }
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj ColorJson(object c)
        {
            var o = new JObj();
            if (c == null) return o;
            foreach (var n in new[] { "r", "g", "b", "a" }) { try { o.Add(n, c.GetType().GetField(n).GetValue(c)); } catch { } }
            return o;
        }

        private static string SpriteName(object sprite)
        {
            if (sprite == null) return null;
            try { return Convert.ToString(sprite.GetType().GetProperty("name").GetValue(sprite), CultureInfo.InvariantCulture); } catch { }
            try { return Convert.ToString(((UnityEngine.Object)sprite).name, CultureInfo.InvariantCulture); } catch { }
            return null;
        }

        private static JObj MenuBackgroundInfo()
        {
            var o = new JObj(); var players = new JArr(); var clips = new JArr(); var rts = new JArr();
            try
            {
                var vpType = FindType("UnityEngine.Video.VideoPlayer");
                if (vpType != null)
                    foreach (var obj in Resources.FindObjectsOfTypeAll(vpType))
                    {
                        var vp = obj as Component;
                        if (vp == null || !vp.gameObject.scene.IsValid()) continue;
                        var e = new JObj().Add("path", UiPath(vp.transform)).Add("name", vp.gameObject.name);
                        foreach (var n in new[] { "isPlaying", "isPaused", "isLooping", "frame", "frameCount", "time", "timeReference", "renderMode", "aspectRatio" })
                            try { e.Add(n, SimpleValue(vp.GetType().GetProperty(n).GetValue(vp))); } catch { }
                        try { var clip = vp.GetType().GetProperty("clip").GetValue(vp) as UnityEngine.Object; e.Add("clip", clip == null ? null : clip.name); } catch { }
                        try { var url = vp.GetType().GetProperty("url").GetValue(vp); e.Add("url", url == null ? null : url.ToString()); } catch { }
                        try { var tex = vp.GetType().GetProperty("targetTexture").GetValue(vp) as Texture; e.Add("targetTexture", tex == null ? null : tex.name); } catch { }
                        players.Add(e);
                    }
                var clipType = FindType("UnityEngine.Video.VideoClip");
                if (clipType != null)
                    foreach (var obj in Resources.FindObjectsOfTypeAll(clipType))
                    {
                        var c = obj as UnityEngine.Object;
                        if (c != null) clips.Add(new JObj().Add("name", c.name).Add("type", c.GetType().Name));
                    }
                foreach (var obj in Resources.FindObjectsOfTypeAll(typeof(RenderTexture)))
                {
                    var rt = obj as RenderTexture;
                    if (rt != null) rts.Add(new JObj().Add("name", rt.name).Add("width", rt.width).Add("height", rt.height));
                }
                o.Add("ok", true).Add("video_players", players).Add("clips", clips).Add("render_textures", rts);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj VideoControl(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                bool pause = req.ContainsKey("pause") && MiniJson.Truthy(req["pause"]);
                var vpType = FindType("UnityEngine.Video.VideoPlayer");
                int n = 0;
                if (vpType != null)
                    foreach (var obj in Resources.FindObjectsOfTypeAll(vpType))
                    {
                        var vp = obj as Component;
                        if (vp == null || !vp.gameObject.scene.IsValid()) continue;
                        try { var m = pause ? vp.GetType().GetMethod("Pause", Type.EmptyTypes) : vp.GetType().GetMethod("Play", Type.EmptyTypes); m?.Invoke(vp, null); n++; } catch { }
                    }
                o.Add("ok", true).Add("paused", pause).Add("players", n);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj UiGeometry()
        {
            var o = new JObj(); var roots = new JArr(); var keys = new JArr();
            try
            {
                var canvasType = FindType("UnityEngine.Canvas");
                var scalerType = FindType("UnityEngine.UI.CanvasScaler");
                var rtuType = FindType("UnityEngine.RectTransformUtility");
                var w2s = rtuType == null ? null : rtuType.GetMethod("WorldToScreenPoint", BindingFlags.Public | BindingFlags.Static, null,
                    new[] { typeof(Camera), typeof(Vector3) }, null);
                var all = Resources.FindObjectsOfTypeAll(canvasType);
                foreach (var obj in all)
                {
                    var cv = obj as Component;
                    if (cv == null || !cv.gameObject.scene.IsValid() || !cv.gameObject.activeInHierarchy) continue;
                    var rt = cv.GetComponent<RectTransform>();
                    if (rt == null) continue;
                    var cam = cv.GetType().GetProperty("worldCamera").GetValue(cv) as Camera;
                    var corners = new Vector3[4]; rt.GetWorldCorners(corners);
                    var sc = new JArr();
                    float minX = 1e9f, maxX = -1e9f, minY = 1e9f, maxY = -1e9f;
                    foreach (var c in corners)
                    {
                        Vector2 p;
                        if (w2s != null) p = (Vector2)w2s.Invoke(null, new object[] { cam, c }); else p = new Vector2(c.x, c.y);
                        sc.Add(new JObj().Add("x", p.x).Add("y", p.y));
                        minX = Math.Min(minX, p.x); maxX = Math.Max(maxX, p.x); minY = Math.Min(minY, p.y); maxY = Math.Max(maxY, p.y);
                    }
                    var r = new JObj().Add("path", UiPath(rt)).Add("name", cv.gameObject.name)
                        .Add("render_mode", Convert.ToString(cv.GetType().GetProperty("renderMode").GetValue(cv), CultureInfo.InvariantCulture))
                        .Add("world_camera", cam == null ? null : cam.name)
                        .Add("sorting_order", cv.GetType().GetProperty("sortingOrder").GetValue(cv))
                        .Add("rect_size", new JObj().Add("x", rt.rect.width).Add("y", rt.rect.height))
                        .Add("size_delta", SimpleValue(rt.sizeDelta)).Add("pivot", SimpleValue(rt.pivot))
                        .Add("anchor_min", SimpleValue(rt.anchorMin)).Add("anchor_max", SimpleValue(rt.anchorMax))
                        .Add("local_scale", SimpleValue(rt.localScale)).Add("position", SimpleValue(rt.position))
                        .Add("world_corners", sc).Add("screen_bbox", new JObj().Add("x0", minX).Add("y0", minY).Add("x1", maxX).Add("y1", maxY));
                    if (scalerType != null)
                    {
                        var scaler = cv.GetComponent(scalerType);
                        if (scaler != null)
                        {
                            var sj = new JObj();
                            foreach (var n in new[] { "uiScaleMode", "referenceResolution", "screenMatchMode", "matchWidthOrHeight", "scaleFactor", "referencePixelsPerUnit" })
                                try { sj.Add(n, SimpleValue(scaler.GetType().GetProperty(n).GetValue(scaler))); } catch { }
                            r.Add("canvas_scaler", sj);
                        }
                    }
                    roots.Add(r);
                }
                foreach (var obj in Resources.FindObjectsOfTypeAll(typeof(RectTransform)))
                {
                    var rt = obj as RectTransform;
                    if (rt == null || !rt.gameObject.activeInHierarchy || !rt.gameObject.scene.IsValid()) continue;
                    string p = UiPath(rt);
                    if (!(p.Contains("ButtonsPanel") || p.Contains("SettingsPanel") || p.Contains("WelcomeScreen") || p.Contains("Main Holder")))
                        continue;
                    var cv = rt.GetComponentInParent(canvasType) as Component;
                    var cam = cv == null ? null : cv.GetType().GetProperty("worldCamera").GetValue(cv) as Camera;
                    var corners = new Vector3[4]; rt.GetWorldCorners(corners);
                    var sc = new JArr(); float minX = 1e9f, maxX = -1e9f, minY = 1e9f, maxY = -1e9f;
                    foreach (var c in corners) { Vector2 q; if (w2s != null) q = (Vector2)w2s.Invoke(null, new object[] { cam, c }); else q = new Vector2(c.x, c.y); sc.Add(new JObj().Add("x", q.x).Add("y", q.y)); minX = Math.Min(minX, q.x); maxX = Math.Max(maxX, q.x); minY = Math.Min(minY, q.y); maxY = Math.Max(maxY, q.y); }
                    keys.Add(new JObj().Add("path", p).Add("rect_size", new JObj().Add("x", rt.rect.width).Add("y", rt.rect.height))
                        .Add("anchored_position", SimpleValue(rt.anchoredPosition)).Add("pivot", SimpleValue(rt.pivot))
                        .Add("world_corners", sc).Add("screen_bbox", new JObj().Add("x0", minX).Add("y0", minY).Add("x1", maxX).Add("y1", maxY)));
                }
                o.Add("ok", true).Add("screen", new JObj().Add("width", Screen.width).Add("height", Screen.height))
                 .Add("canvases", roots).Add("key_rects", keys);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj ButtonTheme()
        {
            var o = new JObj(); var buttons = new JArr(); var images = new JArr(); var texPng = new JObj();
            var pngDone = new HashSet<string>();
            try
            {
                var btnType = FindType("UnityEngine.UI.Button");
                var graphicType = FindType("UnityEngine.UI.Graphic");
                var imgType = FindType("UnityEngine.UI.Image");
                var cgType = FindType("UnityEngine.CanvasGroup");
                var encType = FindType("UnityEngine.ImageConversion");
                var enc = encType?.GetMethod("EncodeToPNG", new[] { typeof(Texture2D) });
                foreach (var obj in Resources.FindObjectsOfTypeAll(btnType))
                {
                    var b = obj as Component;
                    if (b == null || !b.gameObject.activeInHierarchy) continue;
                    string path = UiPath(b.transform);
                    if (!path.Contains("UI Buttons")) continue;
                    var bj = new JObj().Add("path", path).Add("name", b.gameObject.name);
                    try { var g = Mem(b, "targetGraphic") as Component; bj.Add("targetGraphic", g == null ? null : UiPath(g.transform)); if (g != null) bj.Add("target_type", g.GetType().Name); } catch { }
                    try { bj.Add("transition", Convert.ToString(Mem(b, "transition"), CultureInfo.InvariantCulture)); } catch { }
                    try { bj.Add("interactable", Mem(b, "interactable")); } catch { }
                    var colors = Mem(b, "colors");
                    if (colors != null)
                    {
                        var cj = new JObj();
                        foreach (var n in new[] { "normalColor", "highlightedColor", "pressedColor", "selectedColor", "disabledColor",
                                                   "colorMultiplier", "fadeDuration" })
                            try { cj.Add(n, SimpleValue(Mem(colors, n))); } catch { }
                        bj.Add("colors", cj);
                    }
                    var g2 = Mem(b, "targetGraphic") as Component;
                    if (g2 != null)
                    {
                        var gj = new JObj().Add("path", UiPath(g2.transform)).Add("type", g2.GetType().Name);
                        try { gj.Add("color", ColorJson(Mem(g2, "color"))); } catch { }
                        try { gj.Add("raycastTarget", Mem(g2, "raycastTarget")); } catch { }
                        try
                        {
                            var sprite = Mem(g2, "sprite");
                            if (sprite != null)
                            {
                                gj.Add("sprite", SpriteName(sprite));
                                try { var r = Mem(sprite, "rect"); gj.Add("sprite_rect", new JObj().Add("x", Mem(r, "x")).Add("y", Mem(r, "y")).Add("w", Mem(r, "width")).Add("h", Mem(r, "height"))); } catch { }
                                try { gj.Add("sprite_pivot", SimpleValue(Mem(sprite, "pivot"))); } catch { }
                                var tex = Mem(sprite, "texture") as Texture2D;
                                if (tex != null)
                                {
                                    gj.Add("texture", tex.name).Add("texture_width", tex.width).Add("texture_height", tex.height);
                                    if (enc != null && !pngDone.Contains(tex.name))
                                    {
                                        try { var png = (byte[])enc.Invoke(null, new object[] { tex }); texPng.Add(tex.name, Convert.ToBase64String(png)); pngDone.Add(tex.name); } catch { }
                                    }
                                }
                            }
                        }
                        catch { }
                        try
                        {
                            var cr = Mem(g2, "canvasRenderer");
                            if (cr != null) { var gm = cr.GetType().GetMethod("GetColor"); if (gm != null) gj.Add("canvas_renderer_color", ColorJson(gm.Invoke(cr, null))); }
                        }
                        catch { }
                        try
                        {
                            var mat = Mem(g2, "material") as Material;
                            if (mat != null)
                            {
                                gj.Add("material", mat.name).Add("shader", mat.shader == null ? null : mat.shader.name);
                                try { gj.Add("material_color", ColorJson(mat.GetColor("_Color"))); } catch { }
                            }
                        }
                        catch { }
                        var groups = new JArr();
                        var t = g2.transform;
                        while (t != null)
                        {
                            if (cgType != null)
                                foreach (var cg in t.GetComponents(cgType))
                                {
                                    var c = cg as Component;
                                    if (c == null) continue;
                                    groups.Add(new JObj().Add("path", UiPath(c.transform)).Add("alpha", Mem(c, "alpha"))
                                        .Add("interactable", Mem(c, "interactable")).Add("blocks_raycasts", Mem(c, "blocksRaycasts")));
                                }
                            t = t.parent;
                        }
                        gj.Add("canvas_groups", groups);
                        bj.Add("graphic", gj);
                    }
                    buttons.Add(bj);
                }
                if (imgType != null)
                    foreach (var obj in Resources.FindObjectsOfTypeAll(imgType))
                    {
                        var im = obj as Component;
                        if (im == null || !im.gameObject.activeInHierarchy) continue;
                        string path = UiPath(im.transform);
                        if (!path.Contains("UI Buttons")) continue;
                        var ij = new JObj().Add("path", path).Add("name", im.gameObject.name);
                        try { ij.Add("color", ColorJson(Mem(im, "color"))); } catch { }
                        try { var sprite = Mem(im, "sprite"); ij.Add("sprite", SpriteName(sprite)); var tex = sprite == null ? null : Mem(sprite, "texture") as Texture2D; if (tex != null) ij.Add("texture", tex.name); } catch { }
                        images.Add(ij);
                    }
                o.Add("ok", true).Add("buttons", buttons).Add("active_images", images).Add("textures_png", texPng);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static object Mem(object o, string n)
        {
            if (o == null) return null;
            var t = o.GetType();
            var p = t.GetProperty(n, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
            if (p != null) { try { return p.GetValue(o); } catch { } }
            var f = t.GetField(n, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
            if (f != null) { try { return f.GetValue(o); } catch { } }
            return null;
        }

        // The pair adjustments of every TMP font asset (fontFeatureTable.glyphPairAdjustmentRecords): what TMP kerns with. The older
        // kerningTable of FontAssets() is the legacy table and is empty.
        private static JObj FontKerning()
        {
            var o = new JObj(); var arr = new JArr();
            try
            {
                var fontType = FindType("TMPro.TMP_FontAsset");
                if (fontType == null) { o.Add("ok", false).Add("error", "TMP_FontAsset not found"); return o; }
                foreach (var obj in Resources.FindObjectsOfTypeAll(fontType))
                {
                    var font = obj as UnityEngine.Object;
                    if (font == null) continue;
                    var f = new JObj().Add("name", font.name);
                    try
                    {
                        var table = Mem(font, "fontFeatureTable");
                        var records = table == null ? null : Mem(table, "glyphPairAdjustmentRecords") as System.Collections.IEnumerable;
                        var pa = new JArr();
                        int count = 0;
                        if (records != null) foreach (var r in records)
                        {
                            var a = Mem(r, "firstAdjustmentRecord"); var b = Mem(r, "secondAdjustmentRecord");
                            var av = a == null ? null : Mem(a, "glyphValueRecord");
                            var bv = b == null ? null : Mem(b, "glyphValueRecord");
                            pa.Add(new JObj()
                                .Add("a", SimpleValue(a == null ? null : Mem(a, "glyphIndex")))
                                .Add("b", SimpleValue(b == null ? null : Mem(b, "glyphIndex")))
                                .Add("ax", SimpleValue(av == null ? null : Mem(av, "xAdvance")))
                                .Add("apx", SimpleValue(av == null ? null : Mem(av, "xPlacement")))
                                .Add("bx", SimpleValue(bv == null ? null : Mem(bv, "xAdvance")))
                                .Add("bpx", SimpleValue(bv == null ? null : Mem(bv, "xPlacement")))
                                .Add("flags", SimpleValue(Mem(r, "featureLookupFlags"))));
                            count++;
                        }
                        f.Add("pairCount", count).Add("pairs", pa);
                        var chars = Mem(font, "characterTable") as System.Collections.IEnumerable;
                        var ca = new JArr();
                        if (chars != null) foreach (var c in chars)
                            ca.Add(new JObj().Add("unicode", SimpleValue(Mem(c, "m_Unicode"))).Add("glyphIndex", SimpleValue(Mem(c, "m_GlyphIndex"))));
                        f.Add("characterTable", ca);
                    }
                    catch (Exception e) { f.Add("error", e.GetType().Name + ": " + e.Message); }
                    arr.Add(f);
                }
                o.Add("ok", true).Add("fonts", arr).Add("count", arr.Count);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static JObj FontAssets()
        {
            var o = new JObj(); var arr = new JArr();
            try
            {
                var fontType = FindType("TMPro.TMP_FontAsset");
                if (fontType == null) { o.Add("ok", false).Add("error", "TMP_FontAsset not found"); return o; }
                var encType = FindType("UnityEngine.ImageConversion");
                var enc = encType?.GetMethod("EncodeToPNG", new[] { typeof(Texture2D) });
                foreach (var obj in Resources.FindObjectsOfTypeAll(fontType))
                {
                    var font = obj as UnityEngine.Object;
                    if (font == null) continue;
                    var f = new JObj().Add("name", font.name);
                    try
                    {
                        var face = Mem(font, "faceInfo");
                        if (face != null)
                        {
                            var fj = new JObj();
                            foreach (var n in new[] { "pointSize", "scale", "lineHeight", "ascentLine", "descentLine", "unitsPerEM", "unitsPerEm", "padding" })
                                fj.Add(n, SimpleValue(Mem(face, n)));
                            f.Add("faceInfo", fj);
                        }
                    }
                    catch { }
                    try
                    {
                        var chars = Mem(font, "characterTable") as System.Collections.IEnumerable;
                        var ca = new JArr();
                        if (chars != null) foreach (var c in chars)
                        {
                            ca.Add(new JObj().Add("unicode", SimpleValue(Mem(c, "m_Unicode"))).Add("glyphIndex", SimpleValue(Mem(c, "m_GlyphIndex")))
                                .Add("scale", SimpleValue(Mem(c, "m_Scale"))));
                        }
                        f.Add("characterTable", ca);
                    }
                    catch { }
                    try
                    {
                        var glyphs = Mem(font, "glyphTable") as System.Collections.IEnumerable;
                        var ga = new JArr();
                        if (glyphs != null) foreach (var g in glyphs)
                        {
                            var m = Mem(g, "m_Metrics"); var r = Mem(g, "m_GlyphRect");
                            ga.Add(new JObj().Add("index", SimpleValue(Mem(g, "m_Index"))).Add("glyphIndex", SimpleValue(Mem(g, "m_Index")))
                                .Add("scale", SimpleValue(Mem(g, "m_Scale")))
                                .Add("atlasRect", new JObj().Add("x", SimpleValue(Mem(r, "x"))).Add("y", SimpleValue(Mem(r, "y")))
                                    .Add("w", SimpleValue(Mem(r, "width"))).Add("h", SimpleValue(Mem(r, "height"))))
                                .Add("metrics", new JObj().Add("width", SimpleValue(Mem(m, "width"))).Add("height", SimpleValue(Mem(m, "height")))
                                    .Add("bearingX", SimpleValue(Mem(m, "horizontalBearingX"))).Add("bearingY", SimpleValue(Mem(m, "horizontalBearingY")))
                                    .Add("advance", SimpleValue(Mem(m, "horizontalAdvance")))));
                        }
                        f.Add("glyphTable", ga);
                    }
                    catch { }
                    try
                    {
                        var kern = Mem(font, "kerningTable") as System.Collections.IEnumerable;
                        var ka = new JArr();
                        if (kern != null) foreach (var k in kern)
                            ka.Add(new JObj().Add("first", SimpleValue(Mem(k, "m_FirstGlyph"))).Add("second", SimpleValue(Mem(k, "m_SecondGlyph")))
                                .Add("value", SimpleValue(Mem(k, "m_Value"))));
                        f.Add("kerningTable", ka);
                    }
                    catch { }
                    try
                    {
                        var texes = Mem(font, "atlasTextures") as Array;
                        var ta = new JArr();
                        if (texes != null) foreach (var tx in texes)
                        {
                            var t = tx as Texture2D;
                            if (t == null) continue;
                            var tj = new JObj().Add("name", t.name).Add("width", t.width).Add("height", t.height).Add("format", t.format.ToString());
                            if (enc != null) { try { var png = (byte[])enc.Invoke(null, new object[] { t }); tj.Add("png_base64", Convert.ToBase64String(png)); } catch { } }
                            ta.Add(tj);
                        }
                        f.Add("atlasTextures", ta);
                    }
                    catch { }
                    try
                    {
                        var mat = Mem(font, "material") as Material;
                        if (mat != null)
                        {
                            var mj = new JObj().Add("name", mat.name).Add("shader", mat.shader == null ? null : mat.shader.name);
                            foreach (var n in new[] { "_FaceDilate", "_OutlineWidth", "_OutlineSoftness", "_GradientScale", "_ScaleRatioA", "_ScaleRatioB", "_ScaleRatioC", "_UnderlayDilate", "_UnderlaySoftness", "_UnderlayOffsetX", "_UnderlayOffsetY" })
                                try { mj.Add(n, mat.GetFloat(n)); } catch { }
                            try { mj.Add("_FaceColor", SimpleValue(mat.GetColor("_FaceColor"))); } catch { }
                            try { mj.Add("_OutlineColor", SimpleValue(mat.GetColor("_OutlineColor"))); } catch { }
                            try { var tx = mat.GetTexture("_MainTex"); if (tx != null) mj.Add("_MainTex", tx.name).Add("_MainTex_path_id", tx.GetInstanceID()); } catch { }
                            f.Add("material", mj);
                        }
                    }
                    catch { }
                    arr.Add(f);
                }
                o.Add("ok", true).Add("fonts", arr).Add("count", arr.Count);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private static readonly List<KeyValuePair<Component, bool>> FullscreenHidden = new List<KeyValuePair<Component, bool>>();

        private static JObj HideFullscreenGraphics(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                string action = req.ContainsKey("action") ? Convert.ToString(req["action"], CultureInfo.InvariantCulture) : "hide";
                var rootType = FindType("UnityEngine.Canvas");
                GameObject root = null;
                foreach (var go in Resources.FindObjectsOfTypeAll(typeof(GameObject)))
                {
                    var g = go as GameObject;
                    if (g != null && g.scene.IsValid() && g.name == "UI") { root = g; break; }
                }
                if (root == null) { o.Add("ok", false).Add("error", "UI root not found"); return o; }
                if (action == "restore")
                {
                    int n = 0;
                    foreach (var kv in FullscreenHidden) { try { kv.Key.GetType().GetProperty("enabled").SetValue(kv.Key, kv.Value); n++; } catch { } }
                    FullscreenHidden.Clear();
                    o.Add("ok", true).Add("restored", n); return o;
                }
                var imageType = FindType("UnityEngine.UI.Image");
                var rawType = FindType("UnityEngine.UI.RawImage");
                var rtuType = FindType("UnityEngine.RectTransformUtility");
                var worldToScreen = rtuType == null ? null : rtuType.GetMethod("WorldToScreenPoint", BindingFlags.Public | BindingFlags.Static, null,
                    new[] { typeof(Camera), typeof(Vector3) }, null);
                var candidates = new JArr();
                int hidden = 0;
                foreach (var t in new[] { imageType, rawType })
                {
                    if (t == null) continue;
                    foreach (var obj in root.GetComponentsInChildren(t, true))
                    {
                        var c = obj as Component;
                        if (c == null || !c.gameObject.activeInHierarchy) continue;
                        var rt = c.GetComponent<RectTransform>();
                        if (rt == null) continue;
                        try
                        {
                            var color = c.GetType().GetProperty("color").GetValue(c);
                            float alpha = Convert.ToSingle(color.GetType().GetField("a").GetValue(color), CultureInfo.InvariantCulture);
                            if (alpha <= 0f) continue;
                            var canvas = c.GetComponentInParent(FindType("UnityEngine.Canvas")) as Component;
                            var cam = canvas == null ? null : canvas.GetType().GetProperty("worldCamera").GetValue(canvas) as Camera;
                            var corners = new Vector3[4]; rt.GetWorldCorners(corners);
                            float minX = 1e9f, maxX = -1e9f, minY = 1e9f, maxY = -1e9f;
                            foreach (var corner in corners)
                            {
                                Vector2 sp;
                                if (worldToScreen != null) sp = (Vector2)worldToScreen.Invoke(null, new object[] { cam, corner });
                                else sp = new Vector2(corner.x, corner.y);
                                minX = Math.Min(minX, sp.x); maxX = Math.Max(maxX, sp.x);
                                minY = Math.Min(minY, sp.y); maxY = Math.Max(maxY, sp.y);
                            }
                            float w = maxX - minX, h = maxY - minY;
                            bool full = w >= 0.95f * Screen.width && h >= 0.95f * Screen.height
                                        && minX <= 0.05f * Screen.width && maxX >= 0.95f * Screen.width
                                        && minY <= 0.05f * Screen.height && maxY >= 0.95f * Screen.height;
                            var e = new JObj().Add("path", UiPath(rt)).Add("name", c.gameObject.name).Add("type", c.GetType().Name)
                                .Add("alpha", alpha).Add("screen_rect", new JObj().Add("x", minX).Add("y", minY).Add("w", w).Add("h", h))
                                .Add("fullscreen", full);
                            candidates.Add(e);
                            if (full && action == "hide")
                            {
                                var enabledProp = c.GetType().GetProperty("enabled");
                                bool was = (bool)enabledProp.GetValue(c);
                                enabledProp.SetValue(c, false);
                                FullscreenHidden.Add(new KeyValuePair<Component, bool>(c, was));
                                hidden++;
                            }
                        }
                        catch { }
                    }
                }
                o.Add("ok", true).Add("action", action).Add("hidden", hidden).Add("candidates", candidates);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private JObj UiOverlayAll(int w, int h, bool excludeBackground = false)
        {
            var o = new JObj();
            var restores = new List<Action>();
            var canvasRestores = new List<Action>();
            try
            {
                var canvasType = FindType("UnityEngine.Canvas");
                if (canvasType == null) { o.Add("ok", false).Add("error", "Canvas type not found"); return o; }
                Camera ui = FindNamedCamera("UICamera") ?? FindNamedCamera("UI Camera") ?? Camera.main;
                if (ui == null) { o.Add("ok", false).Add("error", "no UI camera"); return o; }
                canvasType.GetMethod("ForceUpdateCanvases", BindingFlags.Public | BindingFlags.Static)?.Invoke(null, null);
                int switched = 0;
                foreach (var obj in Resources.FindObjectsOfTypeAll(canvasType))
                {
                    var cv = obj as Component;
                    if (cv == null || !cv.gameObject.scene.IsValid()) continue;
                    var cvType = cv.GetType();
                    var modeProp = cvType.GetProperty("renderMode");
                    var wcProp = cvType.GetProperty("worldCamera");
                    if (modeProp == null) continue;
                    string mode = Convert.ToString(modeProp.GetValue(cv), CultureInfo.InvariantCulture);
                    if (mode != "ScreenSpaceOverlay" && mode != "ScreenSpaceCamera") continue;
                    if (excludeBackground && (UiPath(cv.transform).ToLowerInvariant().Contains("background") || cv.name == "Title Screen")) continue;
                    var cvLocal = cv; var modeLocal = modeProp; var wcLocal = wcProp;
                    object oldMode = modeProp.GetValue(cv);
                    object oldCam = wcLocal == null ? null : wcLocal.GetValue(cv);
                    canvasRestores.Add(() =>
                    {
                        try { modeLocal.SetValue(cvLocal, oldMode); if (wcLocal != null) wcLocal.SetValue(cvLocal, oldCam); } catch { }
                    });
                    if (mode == "ScreenSpaceOverlay")
                    {
                        var cameraEnum = FindType("UnityEngine.RenderMode");
                        object camMode = Enum.Parse(cameraEnum, "ScreenSpaceCamera");
                        modeProp.SetValue(cv, camMode);
                    }
                    if (wcProp != null) wcProp.SetValue(cv, ui);
                    switched++;
                }
                var tr = ui.transform;
                var oldPos = tr.position; var oldRot = tr.rotation;
                var oldOrtho = ui.orthographic; var oldSize = ui.orthographicSize;
                var oldAspect = ui.aspect; var oldMask = ui.cullingMask;
                var oldClear = ui.clearFlags; var oldColor = ui.backgroundColor;
                var oldTarget = ui.targetTexture; var oldRect = ui.rect;
                restores.Add(() => {
                    tr.position = oldPos; tr.rotation = oldRot;
                    ui.orthographic = oldOrtho; ui.orthographicSize = oldSize;
                    ui.aspect = oldAspect; ui.cullingMask = oldMask;
                    ui.clearFlags = oldClear; ui.backgroundColor = oldColor;
                    ui.targetTexture = oldTarget; ui.rect = oldRect;
                });
                var rt = RenderTexture.GetTemporary(w, h, 24, RenderTextureFormat.ARGB32);
                restores.Add(() => RenderTexture.ReleaseTemporary(rt));
                ui.targetTexture = rt;
                ui.clearFlags = CameraClearFlags.SolidColor;
                ui.backgroundColor = new Color(0f, 0f, 0f, 0f);
                ui.aspect = (float)w / h;
                ui.rect = new Rect(0f, 0f, 1f, 1f);
                ui.orthographic = true;
                ui.orthographicSize = h / 2f;
                ui.cullingMask = -1;
                canvasType.GetMethod("ForceUpdateCanvases", BindingFlags.Public | BindingFlags.Static)?.Invoke(null, null);
                ui.Render();
                RenderTexture.active = rt;
                var tex = new Texture2D(w, h, TextureFormat.RGBA32, false);
                tex.ReadPixels(new Rect(0, 0, w, h), 0, 0);
                tex.Apply(false, false);
                RenderTexture.active = null;
                var encType = FindType("UnityEngine.ImageConversion");
                var enc = encType?.GetMethod("EncodeToPNG", new[] { typeof(Texture2D) });
                if (enc == null) throw new InvalidOperationException("EncodeToPNG not found");
                var png = (byte[])enc.Invoke(null, new object[] { tex });
                var px = tex.GetPixels32();
                int non = 0;
                for (int i = 0; i < px.Length; i++) if (px[i].a != 0) non++;
                UnityEngine.Object.Destroy(tex);
                o.Add("ok", true).Add("camera", ui.name).Add("width", w).Add("height", h)
                 .Add("switched_canvases", switched).Add("nonzero_alpha_pixels", non)
                 .Add("png_base64", Convert.ToBase64String(png));
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            finally
            {
                for (int i = restores.Count - 1; i >= 0; i--) { try { restores[i](); } catch { } }
                for (int i = canvasRestores.Count - 1; i >= 0; i--) { try { canvasRestores[i](); } catch { } }
            }
            return o;
        }

        private static Camera FindNamedCamera(string name)
        {
            foreach (var obj in Resources.FindObjectsOfTypeAll(typeof(Camera)))
            {
                var cam = obj as Camera;
                if (cam != null && cam.gameObject.scene.IsValid() && cam.name == name) return cam;
            }
            return null;
        }

        // Every live bibite's rigid body as Unity holds it (the mass and the rotational inertia the engine actually uses, which the game's C# sets and
        // Unity may recompute from the colliders), with its colliders: what the physics comparison needs.
        private JObj BodiesInfo()
        {
            var o = new JObj(); var arr = new JArr();
            try
            {
                var bodyType = FindType("SimulationScripts.BibiteScripts.BibiteBody");
                if (bodyType == null) { o.Add("ok", false).Add("error", "BibiteBody not found"); return o; }
                foreach (var obj in Resources.FindObjectsOfTypeAll(bodyType))
                {
                    var comp = obj as Component;
                    if (comp == null || !comp.gameObject.scene.IsValid() || comp.hideFlags != HideFlags.None) continue;
                    var b = new JObj();
                    b.Add("id", Convert.ToString(Inst(comp, "id"), CultureInfo.InvariantCulture));
                    var tr = comp.transform;
                    b.Add("pos", SimpleValue(tr.position)).Add("rot_z", tr.eulerAngles.z).Add("scale", SimpleValue(tr.localScale));
                    foreach (var name in new[] { "realBodyArea", "baseBodyArea", "mass", "d1Size", "d2Size", "bodyLength", "baseMass" })
                    {
                        var v = Mem(comp, name);
                        if (v != null) { try { b.Add("body_" + name, Convert.ToDouble(v, CultureInfo.InvariantCulture)); } catch { } }
                    }
                    var rb = comp.GetComponent<Rigidbody2D>();
                    if (rb != null)
                    {
                        var r = new JObj();
                        r.Add("mass", rb.mass).Add("inertia", rb.inertia).Add("useAutoMass", rb.useAutoMass);
                        r.Add("centerOfMass", SimpleValue(rb.centerOfMass)).Add("worldCenterOfMass", SimpleValue(rb.worldCenterOfMass));
                        r.Add("linearDamping", rb.linearDamping).Add("angularDamping", rb.angularDamping);
                        r.Add("velocity", SimpleValue(rb.linearVelocity)).Add("angularVelocity", rb.angularVelocity);
                        r.Add("constraints", rb.constraints.ToString());
                        b.Add("rb", r);
                    }
                    var cols = new JArr();
                    foreach (var col in comp.GetComponentsInChildren<Collider2D>())
                    {
                        var c = new JObj();
                        c.Add("name", col.name).Add("type", col.GetType().Name).Add("trigger", col.isTrigger).Add("enabled", col.enabled);
                        c.Add("offset", SimpleValue(col.offset)).Add("density", col.density).Add("area", col.bounds.size.x * col.bounds.size.y);
                        var cap = col as CapsuleCollider2D; if (cap != null) c.Add("size", SimpleValue(cap.size)).Add("direction", cap.direction.ToString());
                        var circ = col as CircleCollider2D; if (circ != null) c.Add("radius", circ.radius);
                        var bx = col as BoxCollider2D; if (bx != null) c.Add("size", SimpleValue(bx.size));
                        c.Add("lossyScale", SimpleValue(col.transform.lossyScale)).Add("localPos", SimpleValue(col.transform.localPosition));
                        try { c.Add("mass", col.attachedRigidbody != null ? col.attachedRigidbody.mass : 0f); } catch { }
                        // The low-level shapes the engine collides with (vertices in the collider's local space, radius): a capsule is not a capsule there.
                        try
                        {
                            var group = new PhysicsShapeGroup2D();
                            col.GetShapes(group);
                            var shapes = new JArr();
                            for (int i = 0; i < group.shapeCount; i++)
                            {
                                var shape = group.GetShape(i);
                                var verts = new List<Vector2>();
                                group.GetShapeVertices(i, verts);
                                var va = new JArr();
                                foreach (var v in verts) va.Add(new JObj().Add("x", v.x).Add("y", v.y));
                                shapes.Add(new JObj().Add("type", shape.shapeType.ToString()).Add("radius", shape.radius).Add("vertices", va));
                            }
                            c.Add("shapes", shapes);
                        }
                        catch (Exception e) { c.Add("shapes_error", e.GetType().Name + ": " + e.Message); }
                        cols.Add(c);
                    }
                    b.Add("colliders", cols);
                    arr.Add(b);
                }
                o.Add("ok", true).Add("bodies", arr);
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        // Select a live bibite by its runtime id through ManagementScripts.UserControl.SelectBibiteTarget,
        // which opens the normal game inspector panels (equivalent to clicking it in the game UI).
        private JObj SelectBibite(Dictionary<string, object> req)
        {
            var o = new JObj();
            try
            {
                object want = req.ContainsKey("id") ? req["id"] : null;
                var ucType = FindType("ManagementScripts.UserControl");
                var bodyType = FindType("SimulationScripts.BibiteScripts.BibiteBody");
                if (ucType == null || bodyType == null)
                { o.Add("ok", false).Add("error", "UserControl/BibiteBody type not found"); return o; }
                var instField = ucType.GetField("Instance", BindingFlags.Public | BindingFlags.Static);
                var inst = instField == null ? null : instField.GetValue(null);
                if (inst == null) { o.Add("ok", false).Add("error", "UserControl.Instance null"); return o; }
                var select = ucType.GetMethod("SelectBibiteTarget", BindingFlags.Public | BindingFlags.Instance);
                if (select == null) { o.Add("ok", false).Add("error", "SelectBibiteTarget not found"); return o; }
                // Without an id, a request with x / y picks the body nearest to that world position (the first found otherwise).
                bool haveXY = want == null && req.ContainsKey("x") && req.ContainsKey("y");
                double wx = 0, wy = 0;
                if (haveXY)
                {
                    wx = Convert.ToDouble(req["x"], CultureInfo.InvariantCulture);
                    wy = Convert.ToDouble(req["y"], CultureInfo.InvariantCulture);
                }
                Component nearest = null;
                double nearestD = double.MaxValue;
                foreach (var obj in Resources.FindObjectsOfTypeAll(bodyType))
                {
                    var comp = obj as Component;
                    if (comp == null || !comp.gameObject.scene.IsValid() || comp.hideFlags != HideFlags.None) continue;
                    if (haveXY)
                    {
                        var p = comp.transform.position;
                        double d = (p.x - wx) * (p.x - wx) + (p.y - wy) * (p.y - wy);
                        if (d < nearestD) { nearestD = d; nearest = comp; }
                        continue;
                    }
                    object id = Inst(comp, "id");
                    string sid = Convert.ToString(id, CultureInfo.InvariantCulture);
                    bool match = want == null || (Convert.ToString(want, CultureInfo.InvariantCulture) == sid);
                    if (!match) continue;
                    select.Invoke(inst, new object[] { comp });
                    var tr = comp.transform;
                    o.Add("ok", true).Add("selected_id", sid).Add("name", comp.name)
                     .Add("pos", SimpleValue(tr.position)).Add("requested", Convert.ToString(want, CultureInfo.InvariantCulture));
                    return o;
                }
                if (nearest != null)
                {
                    select.Invoke(inst, new object[] { nearest });
                    var tr = nearest.transform;
                    o.Add("ok", true).Add("selected_id", Convert.ToString(Inst(nearest, "id"), CultureInfo.InvariantCulture)).Add("name", nearest.name)
                     .Add("pos", SimpleValue(tr.position)).Add("requested", "xy").Add("distance", Math.Sqrt(nearestD));
                    return o;
                }
                o.Add("ok", false).Add("error", "bibite id not found").Add("requested", Convert.ToString(want, CultureInfo.InvariantCulture));
            }
            catch (Exception e) { o.Add("ok", false).Add("error", e.GetType().Name + ": " + e.Message); }
            return o;
        }

        private JArr UiCameras()
        {
            var arr = new JArr();
            try
            {
                var camType = FindType("UnityEngine.Camera");
                if (camType == null) return arr;
                foreach (var obj in Resources.FindObjectsOfTypeAll(camType))
                {
                    var cam = obj as Camera;
                    if (cam == null || !cam.gameObject.scene.IsValid() || cam.hideFlags != HideFlags.None) continue;
                    arr.Add(new JObj()
                        .Add("name", cam.name)
                        .Add("enabled", cam.enabled)
                        .Add("orthographic", cam.orthographic)
                        .Add("orthographic_size", cam.orthographicSize)
                        .Add("position", SimpleValue(cam.transform.position))
                        .Add("rotation", SimpleValue(cam.transform.rotation))
                        .Add("pixel_rect", SimpleValue(cam.pixelRect))
                        .Add("rect", SimpleValue(cam.rect))
                        .Add("aspect", cam.aspect)
                        .Add("near_clip", cam.nearClipPlane)
                        .Add("far_clip", cam.farClipPlane)
                        .Add("depth", cam.depth)
                        .Add("background", SimpleValue(cam.backgroundColor))
                        .Add("culling_mask", cam.cullingMask));
                }
            }
            catch (Exception e) { arr.Add(new JObj().Add("error", e.GetType().Name + ": " + e.Message)); }
            return arr;
        }

        private JObj UiNode(GameObject go, HashSet<int> visited)
        {
            var o = new JObj();
            o.Add("name", go.name);
            o.Add("active_self", go.activeSelf);
            o.Add("active_in_hierarchy", go.activeInHierarchy);
            o.Add("layer", go.layer);
            o.Add("tag", go.tag);
            try
            {
                var tr = go.transform;
                o.Add("sibling_index", tr.GetSiblingIndex());
                o.Add("child_count", tr.childCount);
                o.Add("path", UiPath(tr));
            }
            catch { }
            var rt = go.GetComponent<RectTransform>();
            if (rt != null)
            {
                o.Add("rect", new JObj()
                    .Add("anchor_min", SimpleValue(rt.anchorMin))
                    .Add("anchor_max", SimpleValue(rt.anchorMax))
                    .Add("anchored_position", SimpleValue(rt.anchoredPosition))
                    .Add("size_delta", SimpleValue(rt.sizeDelta))
                    .Add("pivot", SimpleValue(rt.pivot))
                    .Add("local_position", SimpleValue(rt.localPosition))
                    .Add("local_rotation", SimpleValue(rt.localRotation))
                    .Add("local_scale", SimpleValue(rt.localScale)));
                // final composed screen rect: authoritative world->screen bbox for the composer
                if (go.activeInHierarchy)
                try
                {
                    var canvas = go.GetComponentInParent(FindType("UnityEngine.Canvas"));
                    var cam = canvas == null ? null : canvas.GetType().GetProperty("worldCamera").GetValue(canvas) as Camera;
                    var corners = new Vector3[4];
                    rt.GetWorldCorners(corners);
                    float minX = 1e9f, maxX = -1e9f, minY = 1e9f, maxY = -1e9f;
                    var wc = new JArr();
                    foreach (var c in corners)
                    {
                        Vector2 sp = cam == null ? new Vector2(c.x, c.y) : (Vector2)cam.WorldToScreenPoint(c);
                        wc.Add(new JObj().Add("x", sp.x).Add("y", sp.y).Add("z", c.z));
                        minX = Math.Min(minX, sp.x); maxX = Math.Max(maxX, sp.x);
                        minY = Math.Min(minY, sp.y); maxY = Math.Max(maxY, sp.y);
                    }
                    o.Add("screen_bbox", new JObj().Add("x0", minX).Add("y0", minY).Add("x1", maxX).Add("y1", maxY)
                        .Add("w", maxX - minX).Add("h", maxY - minY).Add("cx", (minX + maxX) / 2f).Add("cy", (minY + maxY) / 2f));
                    o.Add("world_corners", wc);
                }
                catch { }
            }
            var comps = new JArr();
            try
            {
                foreach (var c in go.GetComponents<Component>())
                {
                    if (c == null) continue;
                    var info = UiComponent(c);
                    if (info != null) comps.Add(info);
                }
            }
            catch { }
            o.Add("components", comps);
            var kids = new JArr();
            try
            {
                for (int i = 0; i < go.transform.childCount; i++)
                {
                    var child = go.transform.GetChild(i).gameObject;
                    int id = child.GetInstanceID();
                    if (!visited.Add(id)) continue;
                    kids.Add(UiNode(child, visited));
                }
            }
            catch { }
            if (kids.Count > 0) o.Add("children", kids);
            return o;
        }

        private static string UiPath(Transform tr)
        {
            var parts = new List<string>();
            for (var t = tr; t != null; t = t.parent)
            {
                parts.Insert(0, t.name);
                if (t.parent == null) break;
            }
            return string.Join("/", parts);
        }

        private JObj UiComponent(Component c)
        {
            var t = c.GetType();
            string cn = t.Name;
            var interesting = cn.StartsWith("Image") || cn == "RawImage" || cn == "Text" || cn.Contains("TextMeshPro")
                || cn == "CanvasScaler" || cn.Contains("LayoutGroup") || cn == "LayoutElement"
                || cn == "ContentSizeFitter" || cn == "AspectRatioFitter" || cn == "Canvas"
                || cn == "Button" || cn == "Toggle" || cn == "Slider" || cn == "Scrollbar" || cn == "ScrollRect"
                || cn == "TMP_InputField" || cn == "TMP_Dropdown" || cn == "Dropdown" || cn == "Selectable"
                || cn == "Mask" || cn == "RectMask2D" || cn == "CanvasGroup" || cn == "GraphicRaycaster";
            if (!interesting) return null;
            var o = new JObj();
            o.Add("type", string.IsNullOrEmpty(t.Namespace) ? cn : t.Namespace + "." + cn);
            if (c is Behaviour beh) o.Add("enabled", beh.enabled);
            switch (cn)
            {
                case "Image":
                    AddImage(o, c);
                    break;
                case "RawImage":
                    Add(o, "texture", MemberName(c, "m_Texture", "texture"));
                    Add(o, "color", SimpleValue(Get(c, "m_Color", "color")));
                    Add(o, "uv_rect", SimpleValue(Get(c, "m_UVRect", "uvRect")));
                    break;
                case "Text":
                    Add(o, "text", Get(c, "m_Text", "text"));
                    Add(o, "font", MemberName(c, "m_FontData", "font"));
                    Add(o, "font_size", Get(c, "m_FontSize", "fontSize"));
                    Add(o, "alignment", EnumName(Get(c, "m_TextAnchor", "alignment")));
                    Add(o, "color", SimpleValue(Get(c, "m_Color", "color")));
                    Add(o, "rich_text", Get(c, "m_RichText", "supportRichText"));
                    break;
                case "Canvas":
                    // Canvas is a native component: its values are exposed as managed properties, not
                    // serialized m_* fields, so read both names.
                    Add(o, "render_mode", EnumName(Get(c, "m_RenderMode", "renderMode")));
                    Add(o, "sorting_order", Get(c, "m_SortingOrder", "sortingOrder"));
                    Add(o, "override_sorting", Get(c, "m_OverrideSorting", "overrideSorting"));
                    Add(o, "pixel_perfect", Get(c, "m_PixelPerfect", "pixelPerfect"));
                    Add(o, "plane_distance", Get(c, "m_PlaneDistance", "planeDistance"));
                    Add(o, "target_display", Get(c, "m_TargetDisplay", "targetDisplay"));
                    Add(o, "camera", MemberName(c, "m_Camera", "worldCamera"));
                    Add(o, "sorting_layer_name", Get(c, "m_SortingLayerName", "sortingLayerName"));
                    break;
                case "CanvasScaler":
                    Add(o, "ui_scale_mode", EnumName(Get(c, "m_UiScaleMode")));
                    Add(o, "reference_resolution", SimpleValue(Get(c, "m_ReferenceResolution", "referenceResolution")));
                    Add(o, "screen_match_mode", EnumName(Get(c, "m_ScreenMatchMode", "screenMatchMode")));
                    Add(o, "match_width_or_height", Get(c, "m_MatchWidthOrHeight", "matchWidthOrHeight"));
                    Add(o, "scale_factor", Get(c, "m_ScaleFactor", "scaleFactor"));
                    Add(o, "reference_pixels_per_unit", Get(c, "m_ReferencePixelsPerUnit"));
                    Add(o, "physical_unit", Get(c, "m_PhysicalUnit"));
                    break;
                case "LayoutElement":
                    Add(o, "ignore_layout", Get(c, "m_IgnoreLayout", "ignoreLayout"));
                    Add(o, "min_width", Get(c, "m_MinWidth", "minWidth"));
                    Add(o, "min_height", Get(c, "m_MinHeight", "minHeight"));
                    Add(o, "preferred_width", Get(c, "m_PreferredWidth", "preferredWidth"));
                    Add(o, "preferred_height", Get(c, "m_PreferredHeight", "preferredHeight"));
                    Add(o, "flexible_width", Get(c, "m_FlexibleWidth", "flexibleWidth"));
                    Add(o, "flexible_height", Get(c, "m_FlexibleHeight", "flexibleHeight"));
                    Add(o, "layout_priority", EnumName(Get(c, "m_LayoutPriority")));
                    break;
                case "ContentSizeFitter":
                    Add(o, "horizontal_fit", EnumName(Get(c, "m_HorizontalFit", "horizontalFit")));
                    Add(o, "vertical_fit", EnumName(Get(c, "m_VerticalFit", "verticalFit")));
                    break;
                case "AspectRatioFitter":
                    Add(o, "aspect_mode", EnumName(Get(c, "m_AspectMode", "aspectMode")));
                    Add(o, "aspect_ratio", Get(c, "m_AspectRatio", "aspectRatio"));
                    break;
                case "HorizontalLayoutGroup":
                case "VerticalLayoutGroup":
                case "GridLayoutGroup":
                    AddLayoutGroup(o, c, cn);
                    break;
                case "ScrollRect":
                    Add(o, "horizontal", Get(c, "m_Horizontal", "horizontal"));
                    Add(o, "vertical", Get(c, "m_Vertical", "vertical"));
                    Add(o, "movement_type", EnumName(Get(c, "m_MovementType", "movementType")));
                    Add(o, "elasticity", Get(c, "m_Elasticity", "elasticity"));
                    Add(o, "inertia", Get(c, "m_Inertia", "inertia"));
                    Add(o, "scroll_sensitivity", Get(c, "m_ScrollSensitivity", "scrollSensitivity"));
                    Add(o, "content", MemberName(c, "m_Content", "content"));
                    break;
                case "Toggle":
                    Add(o, "is_on", Get(c, "m_IsOn", "isOn"));
                    Add(o, "interactable", Get(c, "m_Interactable", "interactable"));
                    Add(o, "transition", EnumName(Get(c, "m_Transition", "transition")));
                    break;
                case "Slider":
                    Add(o, "value", Get(c, "m_Value", "value"));
                    Add(o, "min_value", Get(c, "m_MinValue", "minValue"));
                    Add(o, "max_value", Get(c, "m_MaxValue", "maxValue"));
                    Add(o, "whole_numbers", Get(c, "m_WholeNumbers", "wholeNumbers"));
                    Add(o, "interactable", Get(c, "m_Interactable", "interactable"));
                    break;
                case "Scrollbar":
                    Add(o, "value", Get(c, "m_Value", "value"));
                    Add(o, "size", Get(c, "m_Size", "size"));
                    Add(o, "number_of_steps", Get(c, "m_NumberOfSteps", "numberOfSteps"));
                    break;
                case "Selectable":
                case "Button":
                    Add(o, "interactable", Get(c, "m_Interactable", "interactable"));
                    Add(o, "transition", EnumName(Get(c, "m_Transition", "transition")));
                    Add(o, "navigation", EnumName(Get(c, "m_Navigation")));
                    break;
                case "CanvasGroup":
                    Add(o, "alpha", Get(c, "m_Alpha", "alpha"));
                    Add(o, "interactable", Get(c, "m_Interactable", "interactable"));
                    Add(o, "blocks_raycasts", Get(c, "m_BlocksRaycasts", "blocksRaycasts"));
                    Add(o, "ignore_parent_groups", Get(c, "m_IgnoreParentGroups", "ignoreParentGroups"));
                    break;
                default:
                    if (cn.Contains("TextMeshPro"))
                    {
                        Add(o, "text", Get(c, "m_text", "text"));
                        Add(o, "font", MemberName(c, "m_fontAsset", "m_FontAsset", "font"));
                        Add(o, "font_size", Get(c, "m_fontSize", "fontSize"));
                        Add(o, "alignment", EnumName(Get(c, "m_HorizontalAlignment", "m_textAlignment", "alignment")));
                        Add(o, "color", SimpleValue(Get(c, "m_fontColor", "m_color", "color")));
                        Add(o, "word_wrap", Get(c, "m_enableWordWrapping", "m_WordWrapping", "enableWordWrapping"));
                        Add(o, "margin", SimpleValue(Get(c, "m_margin", "margin")));
                        Add(o, "font_style", EnumName(Get(c, "m_fontStyle", "fontStyle")));
                        Add(o, "text_style", Get(c, "m_fontStyle"));
                        Add(o, "rich_text", Get(c, "m_isRichText", "richText"));
                        Add(o, "character_spacing", Get(c, "m_characterSpacing"));
                        Add(o, "enable_kerning", Get(c, "m_enableKerning", "enableKerning"));
                        // What TMP laid out: the first line's ascender / descender / baseline and the first character's, in the text's local
                        // space (the rect's pivot is the origin), to read how its vertical alignment places a line.
                        try
                        {
                            var ti = Get(c, "textInfo");
                            if (ti != null)
                            {
                                var tj = new JObj();
                                tj.Add("lineCount", SimpleValue(Mem(ti, "lineCount")));
                                var lines = Mem(ti, "lineInfo") as Array;
                                if (lines != null && lines.Length > 0)
                                {
                                    var l0 = lines.GetValue(0);
                                    foreach (var n in new[] { "ascender", "descender", "baseline", "lineHeight", "maxAdvance", "width", "length" })
                                        tj.Add("line0_" + n, SimpleValue(Mem(l0, n)));
                                }
                                var chars = Mem(ti, "characterInfo") as Array;
                                if (chars != null && chars.Length > 0)
                                {
                                    var c0 = chars.GetValue(0);
                                    foreach (var n in new[] { "baseLine", "ascender", "descender", "pointSize", "scale", "xAdvance" })
                                        tj.Add("char0_" + n, SimpleValue(Mem(c0, n)));
                                    foreach (var n in new[] { "origin", "topLeft", "bottomLeft" })
                                    {
                                        var v = Mem(c0, n);
                                        tj.Add("char0_" + n, v == null ? "" : v.ToString());
                                    }
                                }
                                o.Add("text_info", tj);
                            }
                        }
                        catch { }
                        Add(o, "line_spacing", Get(c, "m_lineSpacing"));
                    }
                    else
                    {
                        Add(o, "note", "component collected without a specific field map");
                    }
                    break;
            }
            return o;
        }

        private void AddImage(JObj o, Component c)
        {
            var sp = Get(c, "m_Sprite", "sprite");
            o.Add("sprite", MemberName(c, "m_Sprite", "sprite"));
            if (sp is Sprite sprite)
            {
                try
                {
                    o.Add("sprite_rect", SimpleValue(sprite.rect));
                    o.Add("sprite_pixels_per_unit", sprite.pixelsPerUnit);
                    o.Add("sprite_border", SimpleValue(sprite.border));
                }
                catch { }
            }
            Add(o, "color", SimpleValue(Get(c, "m_Color", "color")));
            Add(o, "image_type", EnumName(Get(c, "m_Type", "type")));
            Add(o, "fill_method", EnumName(Get(c, "m_FillMethod", "fillMethod")));
            Add(o, "fill_origin", EnumName(Get(c, "m_FillOrigin", "fillOrigin")));
            Add(o, "fill_amount", Get(c, "m_FillAmount", "fillAmount"));
            Add(o, "fill_clockwise", Get(c, "m_FillClockwise", "fillClockwise"));
            Add(o, "preserve_aspect", Get(c, "m_PreserveAspect", "preserveAspect"));
            Add(o, "use_sprite_mesh", Get(c, "m_UseSpriteMesh", "useSpriteMesh"));
            Add(o, "raycast_target", Get(c, "m_RaycastTarget", "raycastTarget"));
            Add(o, "material", MemberName(c, "m_Material", "material"));
        }

        private void AddLayoutGroup(JObj o, Component c, string cn)
        {
            var pad = Get(c, "m_Padding", "padding");
            if (pad is RectOffset ro)
                o.Add("padding", new JObj().Add("left", ro.left).Add("right", ro.right).Add("top", ro.top).Add("bottom", ro.bottom));
            else
                Add(o, "padding", SimpleValue(pad));
            Add(o, "spacing", SimpleValue(Get(c, "m_Spacing", "spacing")));
            Add(o, "child_alignment", EnumName(Get(c, "m_ChildAlignment", "childAlignment")));
            Add(o, "reverse_arrangement", Get(c, "m_ReverseArrangement", "reverseArrangement"));
            Add(o, "child_control_width", Get(c, "m_ChildControlWidth", "childControlWidth"));
            Add(o, "child_control_height", Get(c, "m_ChildControlHeight", "childControlHeight"));
            Add(o, "child_scale_width", Get(c, "m_ChildScaleWidth", "childScaleWidth"));
            Add(o, "child_scale_height", Get(c, "m_ChildScaleHeight", "childScaleHeight"));
            Add(o, "child_force_expand_width", Get(c, "m_ChildForceExpandWidth", "childForceExpandWidth"));
            Add(o, "child_force_expand_height", Get(c, "m_ChildForceExpandHeight", "childForceExpandHeight"));
            if (cn == "GridLayoutGroup")
            {
                Add(o, "cell_size", SimpleValue(Get(c, "m_CellSize", "cellSize")));
                Add(o, "constraint", EnumName(Get(c, "m_Constraint", "constraint")));
                Add(o, "constraint_count", Get(c, "m_ConstraintCount", "constraintCount"));
                Add(o, "start_corner", EnumName(Get(c, "m_StartCorner", "startCorner")));
                Add(o, "start_axis", EnumName(Get(c, "m_StartAxis", "startAxis")));
            }
        }

        private const BindingFlags UiFlags = BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic;

        private static object Get(object o, params string[] names)
        {
            if (o == null) return null;
            foreach (var n in names)
            {
                try
                {
                    var p = o.GetType().GetProperty(n, UiFlags);
                    if (p != null && p.CanRead && p.GetIndexParameters().Length == 0) return p.GetValue(o);
                    var f = o.GetType().GetField(n, UiFlags);
                    if (f != null) return f.GetValue(o);
                }
                catch { }
            }
            return null;
        }

        private static object MemberName(object o, params string[] names)
        {
            var v = Get(o, names);
            if (v is UnityEngine.Object uo) return uo.name;
            return v == null ? null : Convert.ToString(v, CultureInfo.InvariantCulture);
        }

        private static string EnumName(object v)
        {
            return v == null ? null : (v.GetType().IsEnum ? v.ToString() : Convert.ToString(v, CultureInfo.InvariantCulture));
        }

        private static void Add(JObj o, string key, object v)
        {
            if (v == null) return;
            var sv = SimpleValue(v);
            if (sv != null) o.Add(key, sv);
        }

    }

    // =================================================================================================
    // Exact feeding-energy accumulators (Harmony hooks, task-94 completion)
    // =================================================================================================
    internal static class FeedingPatches
    {
        public static double Produced, Consumed, Recycled, Decayed, Culled;
        public static readonly Dictionary<int, double> ZoneProduced = new Dictionary<int, double>();
        public static readonly Dictionary<int, double> ZoneConsumed = new Dictionary<int, double>();
        public static readonly Dictionary<int, double> ZoneRecycled = new Dictionary<int, double>();
        public static readonly Dictionary<int, double> ZoneDecayed = new Dictionary<int, double>();
        public static readonly Dictionary<int, double> ZoneCulled = new Dictionary<int, double>();
        public static readonly Dictionary<int, int> PelletZone = new Dictionary<int, int>();
        private static bool _inRecycle;
        private static bool _inDecay;
        private static readonly BindingFlags Flags = BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic;

        public static void ResetExact()
        {
            Produced = Consumed = Recycled = Decayed = Culled = 0;
            ZoneProduced.Clear(); ZoneConsumed.Clear(); ZoneRecycled.Clear(); ZoneDecayed.Clear(); ZoneCulled.Clear();
        }

        public static double ZoneValue(Dictionary<int, double> d, int zone) { return d.TryGetValue(zone, out var v) ? v : 0.0; }
        private static void Add(Dictionary<int, double> d, int zone, double v) { if (zone >= 0) d[zone] = ZoneValue(d, zone) + v; }
        private static double Energy(object pellet) { try { return Convert.ToDouble(Inst(pellet, "energy"), CultureInfo.InvariantCulture); } catch { return 0.0; } }
        private static int ZoneOf(object pellet)
        {
            try
            {
                int id = ((UnityEngine.Object)pellet).GetInstanceID();
                return PelletZone.TryGetValue(id, out var z) ? z : -1;
            }
            catch { return -1; }
        }
        private static object Inst(object instance, string field)
        {
            if (instance == null) return null;
            for (var t = instance.GetType(); t != null; t = t.BaseType)
            {
                var f = t.GetField(field, Flags);
                if (f != null) return f.GetValue(instance);
            }
            return null;
        }
        private static int ZoneIndex(object zone)
        {
            try { return ((UnityEngine.Object)zone).GetInstanceID(); } catch { return -1; }
        }

        public static void SpawnPostfix(object __instance, float __result)
        {
            double v = Math.Max(0f, __result);
            Produced += v;
            Add(ZoneProduced, ZoneIndex(__instance), v);
        }

        private static double StomachAmount(object mouth)
        {
            try { return Convert.ToDouble(PropVal(Inst(mouth, "stomach"), "totalAmount"), CultureInfo.InvariantCulture); }
            catch { return 0.0; }
        }
        private static List<object> AsList(object o)
        {
            var list = new List<object>();
            if (o is System.Collections.IEnumerable en) foreach (var x in en) list.Add(x);
            return list;
        }
        private static double PelletListEnergy(object listObj)
        {
            double sum = 0.0;
            foreach (var p in AsList(listObj)) { if (p != null) sum += Math.Max(0.0, Energy(p)); }
            return sum;
        }
        private static object PropVal(object instance, string prop)
        {
            try
            {
                if (instance == null) return null;
                var p = instance.GetType().GetProperty(prop, Flags);
                return p != null && p.CanRead ? p.GetValue(instance) : null;
            }
            catch { return null; }
        }

        // consumption: MaybeAttack (large method, not inlined) swallows pellet chunks into the stomach
        private static double _recycleBefore;
        private static double _decayBefore;

        public static void AttackPrefix(object __instance, out double __state) { __state = StomachAmount(__instance); }

        public static void AttackPostfix(object __instance, double __state)
        {
            double delta = StomachAmount(__instance) - __state;
            if (delta <= 0) return;
            double density = 5.0;
            try
            {
                var pellet = Inst(__instance, "targetPellet");
                var mat = pellet == null ? null : Inst(pellet, "material");
                double d = Convert.ToDouble(Inst(mat, "EnergyDensity"), CultureInfo.InvariantCulture);
                if (d > 0) density = d;
            }
            catch { }
            double e = delta * density;
            Consumed += e;
            Add(ZoneConsumed, -1, e);
        }

        public static void RecyclePrefix(object __instance) { _recycleBefore = PelletListEnergy(Inst(__instance, "zonePellets")); }
        public static void RecyclePostfix(object __instance)
        {
            double delta = _recycleBefore - PelletListEnergy(Inst(__instance, "zonePellets"));
            if (delta > 0) { Recycled += delta; Add(ZoneRecycled, -1, delta); }
        }

        public static void DecayPrefix(object __instance) { _decayBefore = PelletListEnergy(Inst(__instance, "pellets")); }
        public static void DecayPostfix(object __instance)
        {
            double delta = _decayBefore - PelletListEnergy(Inst(__instance, "pellets"));
            if (delta > 0) { Decayed += delta; Add(ZoneDecayed, -1, delta); }
        }

        // ---- mouth-contact persistence metrics (task-105) ---------------------------------------
        private sealed class ContactState
        {
            public int LastN;
            public long ContactTicks, PlantTicks, EnterEvents, ExitEvents;
            public long FirstTick = -1, FirstPlantTick = -1, LastContactTick = -1;
        }
        private static readonly Dictionary<int, ContactState> Contacts = new Dictionary<int, ContactState>();

        private static long SimTick()
        {
            try { return (long)Math.Round(Plugin.GetSimulatedTime() / Math.Max(1e-9, Time.fixedDeltaTime)); }
            catch { return -1; }
        }

        public static int NMouth(object mouth)
        {
            try { return mouth == null ? 0 : Convert.ToInt32(Inst(mouth, "nInMouth"), CultureInfo.InvariantCulture); }
            catch { return 0; }
        }

        private static object PelletInMouth(object mouth, out bool anyPlant, out bool anyMeat, out int count)
        {
            anyPlant = anyMeat = false; count = 0; object first = null;
            var objs = Inst(mouth, "objectsInMouth") as Array;
            if (objs == null) return null;
            var pelletType = Plugin.FindType("SimulationScripts.MatterPellet");
            for (int i = 0; i < objs.Length; i++)
            {
                var go = objs.GetValue(i) as GameObject;
                if (go == null) continue;
                count++;
                var mp = pelletType == null ? null : go.GetComponent(pelletType);
                if (mp != null)
                {
                    if (first == null) first = mp;
                    string mat = MatName(Inst(mp, "material"));
                    if (mat.Contains("plant")) anyPlant = true;
                    else if (mat.Contains("meat")) anyMeat = true;
                }
            }
            return first;
        }

        private static string MatName(object mat)
        {
            object n = null;
            try { n = mat == null ? null : mat.GetType().GetProperty("Name", Flags)?.GetValue(mat); } catch { }
            if (n == null) try { n = mat == null ? null : mat.GetType().GetProperty("name", Flags)?.GetValue(mat); } catch { }
            string s = n as string;
            return string.IsNullOrEmpty(s) ? "" : s.ToLowerInvariant();
        }

        public static JArr MouthTargets(object mouth)
        {
            var arr = new JArr();
            var objs = Inst(mouth, "objectsInMouth") as Array;
            if (objs == null) return arr;
            var pelletType = Plugin.FindType("SimulationScripts.MatterPellet");
            for (int i = 0; i < objs.Length; i++)
            {
                var go = objs.GetValue(i) as GameObject;
                if (go == null) continue;
                var mp = pelletType == null ? null : go.GetComponent(pelletType);
                string kind = "bibite";
                object amount = null, held = null;
                if (mp != null)
                {
                    string mat = MatName(Inst(mp, "material"));
                    kind = mat.Contains("plant") ? "plant" : (mat.Contains("meat") ? "meat" : "matter");
                    amount = PropVal(mp, "amount");
                    held = Inst(mp, "held");
                }
                arr.Add(new JObj().Add("kind", kind).Add("tag", go.tag).Add("amount", amount).Add("held", held));
            }
            return arr;
        }

        public static long ContactValue(object mouth, string which)
        {
            if (mouth == null) return 0;
            int id = ((UnityEngine.Object)mouth).GetInstanceID();
            ContactState st;
            lock (Contacts)
            {
                if (!Contacts.TryGetValue(id, out st)) return which == "first" || which == "first_plant" ? -1 : 0;
            }
            switch (which)
            {
                case "contact": return st.ContactTicks;
                case "plant": return st.PlantTicks;
                case "enter": return st.EnterEvents;
                case "first": return st.FirstTick;
                case "first_plant": return st.FirstPlantTick;
                default: return 0;
            }
        }

        public static long ContactTotal(string which)
        {
            long sum = 0;
            lock (Contacts)
            {
                foreach (var st in Contacts.Values)
                    sum += which == "contact" ? st.ContactTicks : which == "plant" ? st.PlantTicks : st.EnterEvents;
            }
            return sum;
        }

        private static void UpdateContact(object mouth)
        {
            if (mouth == null) return;
            int id = ((UnityEngine.Object)mouth).GetInstanceID();
            int n = NMouth(mouth);
            bool plant, meat; int count;
            PelletInMouth(mouth, out plant, out meat, out count);
            long tick = SimTick();
            lock (Contacts)
            {
                ContactState st;
                if (!Contacts.TryGetValue(id, out st)) { st = new ContactState(); Contacts[id] = st; }
                if (n > 0)
                {
                    st.ContactTicks++;
                    if (plant) st.PlantTicks++;
                    if (st.LastN == 0)
                    {
                        st.EnterEvents++;
                        if (st.FirstTick < 0) st.FirstTick = tick;
                        if (plant && st.FirstPlantTick < 0) st.FirstPlantTick = tick;
                    }
                    st.LastContactTick = tick;
                }
                else if (st.LastN > 0)
                {
                    st.ExitEvents++;
                }
                st.LastN = n;
            }
        }

        public static void MouthUpdatePostfix(object __instance) { UpdateContact(__instance); }
        public static void MouthEnterPostfix(object __instance) { UpdateContact(__instance); }
        public static void MouthExitPostfix(object __instance) { UpdateContact(__instance); }

        // ---- per-tick trajectory/command trace (task-122) ---------------------------------------
        private static StreamWriter _trace;
        private static int _traceRows;
        private static int _traceCalls;
        private static int _collisionCalls;
        private static int _collisionContacts;
        private static StreamWriter _physics2;
        private static string Physics2Path;
        private static string _traceErrors;
        private static int _plantFrame = -1;
        private static List<Vector2> _plantPositions = new List<Vector2>();
        private static readonly MethodInfo _brainOutput = FindBrainOutput();
        private static int _brainOutputParam;

        private static MethodInfo FindBrainOutput()
        {
            try
            {
                var outType = Plugin.FindType("SimulationScripts.BibiteScripts.NEATBrain+Outputs");
                var brainType = Plugin.FindType("SimulationScripts.BibiteScripts.NEATBrain");
                if (outType == null || brainType == null) return null;
                var m = brainType.GetMethod("Output", BindingFlags.Public | BindingFlags.Instance, null,
                                            new[] { outType }, null);
                _brainOutputParam = 0;
                return m;
            }
            catch { return null; }
        }

        private static Harmony _traceHarmony;
        public static bool TraceHooked { get; private set; }

        private static void EnsureTraceHook()
        {
            if (TraceHooked) return;
            try
            {
                var bodyType = Plugin.FindType("SimulationScripts.BibiteScripts.BibiteBody");
                if (bodyType == null) return;
                var m = AccessTools.Method(bodyType, "FixedUpdate", Type.EmptyTypes);
                if (m == null) m = AccessTools.Method(bodyType, "Update", Type.EmptyTypes);
                if (m == null) return;
                _traceHarmony = _traceHarmony ?? new Harmony("rustibites.parity.trace");
                _traceHarmony.Patch(m, prefix: new HarmonyMethod(typeof(FeedingPatches).GetMethod("TraceBodyPrefix")),
                                    postfix: new HarmonyMethod(typeof(FeedingPatches).GetMethod("TraceBodyPostfix")));
                var partType = Plugin.FindType("SimulationScripts.BibiteScripts.BibitePart");
                var collision = partType == null ? null
                    : AccessTools.Method(partType, "OnCollisionEnter2D", new[] { typeof(Collision2D) });
                if (collision != null)
                    _traceHarmony.Patch(collision, postfix: new HarmonyMethod(typeof(FeedingPatches).GetMethod("PartCollisionPostfix")));
                foreach (var cname in new[] { "OnCollisionStay2D", "OnCollisionExit2D" })
                {
                    var cm = partType == null ? null : AccessTools.Method(partType, cname, new[] { typeof(Collision2D) });
                    if (cm != null)
                        _traceHarmony.Patch(cm, postfix: new HarmonyMethod(typeof(FeedingPatches).GetMethod("PartCollisionPostfix")));
                }
                TraceHooked = true;
            }
            catch { }
        }

        public static void TraceStart(string path)
        {
            try
            {
                EnsureTraceHook();
                TraceStop();
                if (string.IsNullOrEmpty(path)) return;
                var dir = Path.GetDirectoryName(path);
                if (!string.IsNullOrEmpty(dir)) Directory.CreateDirectory(dir);
                _trace = new StreamWriter(new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.Read));
                _trace.AutoFlush = true;
                _trace.WriteLine("{\"trace_header\":1,\"tick\":" + SimTick() + "}");
                _traceRows = 0;
                _traceCalls = 0;
                _collisionCalls = 0;
                _collisionContacts = 0;
                _traceErrors = null;
                PhysicsInfoPath = path + ".physics.json";
                Physics2Path = path + ".physics2.jsonl";
                try { _physics2 = new StreamWriter(new FileStream(Physics2Path, FileMode.Create, FileAccess.Write, FileShare.Read)); _physics2.AutoFlush = false; _physics2.WriteLine("{\"type\":\"header\",\"schema\":\"rustibites.parity.physics2.v1\"}"); } catch { }
                try { WriteColliderCensus(); } catch (Exception e) { try { File.AppendAllText(Physics2Path + ".err", e.ToString()); } catch { } }
                WritePhysicsInfo();
                _plantFrame = -1;
            }
            catch (Exception e) { Console.Error.WriteLine("trace start failed: " + e.Message); }
        }

        public static int TraceCallCount => _traceCalls;
        public static int CollisionCallCount => _collisionCalls;
        public static int CollisionContactCount => _collisionContacts;

        private static string PhysicsInfoPath;

        private static void P2(JObj o)
        {
            try { if (_physics2 != null) _physics2.WriteLine(Json.Write(o)); } catch { }
        }

        private static object Fld(object o, string name)
        {
            try
            {
                if (o == null) return null;
                var t = o.GetType();
                var f = t.GetField(name); if (f != null) return f.GetValue(o);
                var p = t.GetProperty(name); return p != null && p.CanRead ? p.GetValue(o) : null;
            }
            catch { return null; }
        }

        private static JObj Vec3(object v)
        {
            try
            {
                var t = v.GetType();
                object g(string n) { var f = t.GetField(n); return f != null ? f.GetValue(v) : t.GetProperty(n).GetValue(v); }
                return new JObj().Add("x", g("x")).Add("y", g("y")).Add("z", g("z"));
            }
            catch { return null; }
        }

        private static JObj Vec2(object v)
        {
            try
            {
                var f = v.GetType().GetField("x"); var p = v.GetType().GetProperty("x");
                var fx = f != null ? f.GetValue(v) : p.GetValue(v);
                var fy = f != null ? v.GetType().GetField("y").GetValue(v) : v.GetType().GetProperty("y").GetValue(v);
                return new JObj().Add("x", fx).Add("y", fy);
            }
            catch { return null; }
        }

        private static void WriteColliderCensus()
        {
            try
            {
                var bodyType = Plugin.FindType("SimulationScripts.BibiteScripts.BibiteBody");
                var colType = typeof(Collider2D);
                var bodies = bodyType == null ? null : UnityEngine.Object.FindObjectsOfType(bodyType);
                P2(new JObj().Add("type", "census_debug").Add("body_type", bodyType == null ? null : bodyType.FullName)
                    .Add("bodies", bodies == null ? 0 : bodies.Length));
                if (bodies == null) return;
                foreach (var b in bodies)
                {
                    var comp = b as Component;
                    if (comp == null) continue;
                    int id = 0;
                    try { id = IDv(Inst(Inst(comp, "id"), "id")); } catch { }
                    var cols = comp.GetComponentsInChildren(colType);
                    P2(new JObj().Add("type", "census_body").Add("id", id).Add("name", comp.name)
                        .Add("collider_components", cols == null ? 0 : cols.Length));
                    if (cols == null) continue;
                    int idx = 0;
                    foreach (var col in cols)
                    {
                        var c = col as Component;
                        if (c == null) continue;
                        var o = new JObj().Add("type", "census").Add("id", id).Add("part", idx)
                            .Add("collider", c.GetType().Name).Add("name", c.gameObject.name)
                            .Add("layer", c.gameObject.layer).Add("is_trigger", Fld(c, "isTrigger"))
                            .Add("enabled", Fld(c, "enabled"));
                        try
                        {
                            var lpos = Vec3(Fld(c.transform, "localPosition")); if (lpos != null) o.Add("local_position", lpos);
                            var lscl = Vec3(Fld(c.transform, "localScale")); if (lscl != null) o.Add("local_scale", lscl);
                            var lrot = Fld(c.transform, "localRotation");
                            if (lrot != null)
                            {
                                var q = Vec3(Fld(lrot, "eulerAngles")); if (q != null) o.Add("local_rotation_euler", q);
                            }
                            var lEuler = Fld(c.transform, "localEulerAngles"); if (lEuler != null) o.Add("local_euler", Vec3(lEuler));
                            var wRot = Fld(c.transform, "eulerAngles"); if (wRot != null) o.Add("world_rotation", Vec3(wRot));
                            var bodyScale = Vec3(Fld(comp.transform, "localScale")); if (bodyScale != null) o.Add("body_scale", bodyScale);
                            var bodyRot = Vec3(Fld(comp.transform, "eulerAngles")); if (bodyRot != null) o.Add("body_rotation", bodyRot);
                        }
                        catch { }
                        try
                        {
                            var size = Vec2(Fld(c, "size")); if (size != null) o.Add("size", size);
                            var radius = Fld(c, "radius"); if (radius != null) o.Add("radius", radius);
                            var dir = Fld(c, "direction"); if (dir != null) o.Add("direction", dir);
                            var ppu = Fld(c, "density"); if (ppu != null) o.Add("density", ppu);
                            var offset = Vec2(Fld(c, "offset")); if (offset != null) o.Add("offset", offset);
                            var bounds = Fld(c, "bounds");
                            if (bounds != null)
                            {
                                var bc = Vec2(Fld(bounds, "center")); if (bc != null) o.Add("bounds_center", bc);
                                var bs = Vec2(Fld(bounds, "size")); if (bs != null) o.Add("bounds_size", bs);
                            }
                        }
                        catch { }
                        P2(o);
                        idx++;
                    }
                }
            }
            catch (Exception e)
            {
                try { File.AppendAllText(Physics2Path + ".err", e.ToString()); } catch { }
            }
        }

        private static int QueryPersistent(object body, Component comp, out double impulseSum, out int count)
        {
            impulseSum = 0; count = 0;
            try
            {
                var cpType = typeof(ContactPoint2D);
                var colType = typeof(Collider2D);
                var filterType = Plugin.FindType("UnityEngine.ContactFilter2D");
                if (cpType == null || colType == null || comp == null) return 0;
                int id = IDv(Inst(Inst(comp, "id"), "id"));
                int idx = 0;
                foreach (var col in comp.GetComponentsInChildren(colType))
                {
                    var c = col as Component;
                    if (c == null) continue;
                    var arr = Array.CreateInstance(cpType, 32);
                    object[] args = null;
                    var getter = c.GetType().GetMethod("GetContacts", new[] { arr.GetType() });
                    if (getter != null) args = new object[] { arr };
                    else if (filterType != null)
                    {
                        var filter = Activator.CreateInstance(filterType);
                        try { filterType.GetMethod("NoFilter")?.Invoke(filter, null); } catch { }
                        getter = c.GetType().GetMethod("GetContacts", new[] { filterType, arr.GetType() });
                        if (getter != null) args = new object[] { filter, arr };
                    }
                    int n = getter == null ? 0 : Convert.ToInt32(getter.Invoke(c, args), CultureInfo.InvariantCulture);
                    try
                    {
                        var q = new JObj().Add("type", "query").Add("tick", SimTick()).Add("id", id)
                            .Add("part", idx).Add("part_name", c.gameObject.name)
                            .Add("collider", c.GetType().Name).Add("contacts", n)
                            .Add("is_trigger", Fld(c, "isTrigger")).Add("enabled", Fld(c, "enabled"));
                        var lp = Vec3(Fld(c.transform, "localPosition")); if (lp != null) q.Add("local_position", lp);
                        var ls = Vec3(Fld(c.transform, "localScale")); if (ls != null) q.Add("local_scale", ls);
                        var le = Fld(c.transform, "localEulerAngles"); if (le != null) q.Add("local_euler", Vec3(le));
                        var wr = Fld(c.transform, "eulerAngles"); if (wr != null) q.Add("world_rotation", Vec3(wr));
                        var dir = Fld(c, "direction"); if (dir != null) q.Add("direction", dir);
                        var bd = Fld(c, "bounds");
                        if (bd != null) { var bs = Vec2(Fld(bd, "size")); if (bs != null) q.Add("bounds_size", bs); }
                        P2(q);
                    }
                    catch { }
                    for (int i = 0; i < n; i++)
                    {
                        object cp = arr.GetValue(i);
                        if (cp == null) continue;
                        var point = Vec2(Fld(cp, "point"));
                        var normal = Vec2(Fld(cp, "normal"));
                        var relvel = Vec2(Fld(cp, "relativeVelocity"));
                        double imp = 0;
                        try { imp = Convert.ToDouble(Fld(cp, "normalImpulse"), CultureInfo.InvariantCulture); } catch { }
                        impulseSum += Math.Abs(imp); count++;
                        var pelletType = Plugin.FindType("SimulationScripts.MatterPellet");
                        var aComp = Fld(cp, "collider") as Component;
                        var bComp = Fld(cp, "otherCollider") as Component;
                        Component pelletComp = null, partComp = null;
                        object pellet = null;
                        if (pelletType != null)
                        {
                            if (aComp != null) pellet = aComp.GetComponent(pelletType);
                            if (pellet != null) { pelletComp = aComp; partComp = bComp; }
                            else if (bComp != null) { pellet = bComp.GetComponent(pelletType); if (pellet != null) { pelletComp = bComp; partComp = aComp; } }
                        }
                        if (pelletComp == null) { pelletComp = bComp; partComp = aComp; }
                        long pelletHandle = pelletComp == null ? 0 : pelletComp.gameObject.GetInstanceID();
                        string pelletName = pelletComp == null ? null : pelletComp.gameObject.name;
                        int pelletId = 0;
                        try { pelletId = IDv(Inst(Inst(pellet, "id"), "id")); } catch { }
                        object pelletAmount = pellet == null ? null : Fld(pellet, "amount");
                        string pelletMat = pellet == null ? null : MatName(Inst(pellet, "material"));
                        long tickNow = SimTick();
                        string contactKey = id + "|" + idx + "|" + pelletHandle;
                        int lastTick;
                        bool persistent = LastContactTick.TryGetValue(contactKey, out lastTick) && lastTick == tickNow - 1;
                        LastContactTick[contactKey] = (int)tickNow;
                        var o = new JObj().Add("type", "contact").Add("tick", tickNow).Add("t", Plugin.GetSimulatedTime())
                            .Add("id", id).Add("part", idx).Add("part_name", c.gameObject.name)
                            .Add("collider", c.GetType().Name)
                            .Add("persistent", persistent)
                            .Add("contact_collider", partComp == null ? null : partComp.gameObject.name)
                            .Add("other_collider", pelletName).Add("other_layer", pelletComp == null ? null : pelletComp.gameObject.layer)
                            .Add("pellet_handle", pelletHandle).Add("pellet_id", pelletId)
                            .Add("pellet_material", pelletMat).Add("pellet_amount", pelletAmount)
                            .Add("normal_impulse", imp).Add("separation", Fld(cp, "separation"));
                        if (point != null) o.Add("point", point);
                        if (normal != null) o.Add("normal", normal);
                        if (relvel != null) o.Add("relative_velocity", relvel);
                        P2(o);
                    }
                    idx++;
                }
            }
            catch { }
            return count;
        }
        private static void WritePhysicsInfo()
        {
            try
            {
                var o = new JObj();
                var layers = new JArr();
                for (int i = 0; i < 32; i++) layers.Add(LayerMask.LayerToName(i));
                o.Add("layer_names", layers);
                var ignored = new JArr();
                for (int i = 0; i < 32; i++)
                    for (int j = i + 1; j < 32; j++)
                        if (Physics2D.GetIgnoreLayerCollision(i, j))
                            ignored.Add(new JArr().Add(i).Add(j).Add(LayerMask.LayerToName(i)).Add(LayerMask.LayerToName(j)));
                o.Add("ignored_layer_pairs", ignored);
                var cols = new JArr();
                var counts = new Dictionary<string, int>();
                var colType = typeof(Collider2D);
                if (colType != null)
                {
                    foreach (var obj in UnityEngine.Object.FindObjectsOfType(colType))
                    {
                        var c = obj as Collider2D;
                        if (c == null || !c.gameObject.scene.IsValid()) continue;
                        bool trigger = false;
                        try { trigger = c.isTrigger; } catch { }
                        string key = "layer=" + c.gameObject.layer + "(" + LayerMask.LayerToName(c.gameObject.layer) + ")"
                                     + " trigger=" + trigger + " type=" + c.GetType().Name;
                        counts[key] = (counts.ContainsKey(key) ? counts[key] : 0) + 1;
                        if (cols.Count < 400)
                            cols.Add(new JObj().Add("name", c.gameObject.name)
                                .Add("type", c.GetType().Name).Add("layer", c.gameObject.layer)
                                .Add("layer_name", LayerMask.LayerToName(c.gameObject.layer))
                                .Add("is_trigger", trigger).Add("enabled", c.enabled)
                                .Add("tag", c.gameObject.tag).Add("body", c.attachedRigidbody == null ? null : c.attachedRigidbody.name));
                    }
                }
                var cc = new JObj();
                foreach (var kv in counts) cc.Add(kv.Key, kv.Value);
                o.Add("collider_counts", cc);
                o.Add("colliders", cols);
                o.Add("collision_callback_calls", _collisionCalls);
                o.Add("collision_callback_contacts", _collisionContacts);
                if (!string.IsNullOrEmpty(PhysicsInfoPath))
                    File.WriteAllText(PhysicsInfoPath, Json.Write(o));
            }
            catch { }
        }
        public static string TraceError => _traceErrors;

        public static void TraceStop()
        {
            try
            {
                WritePhysicsInfo();
                try { if (_physics2 != null) { _physics2.Flush(); _physics2.Dispose(); _physics2 = null; } } catch { }
                if (_trace != null) { _trace.Flush(); _trace.Dispose(); _trace = null; }
            }
            catch { }
        }

        private static void EnsurePlants()
        {
            int frame = Time.frameCount;
            if (frame == _plantFrame) return;
            _plantFrame = frame;
            _plantPositions.Clear();
            try
            {
                var pelletType = Plugin.FindType("SimulationScripts.MatterPellet");
                if (pelletType == null) return;
                foreach (var obj in UnityEngine.Object.FindObjectsOfType(pelletType))
                {
                    var mp = obj as Component;
                    if (mp == null || !mp.gameObject.scene.IsValid()) continue;
                    string mat = MatName(Inst(mp, "material"));
                    if (!mat.Contains("plant")) continue;
                    _plantPositions.Add(mp.transform.position);
                }
            }
            catch { }
        }

        private static float BrainValue(object brain, string name)
        {
            try
            {
                if (brain == null || _brainOutput == null) return float.NaN;
                var outType = _brainOutput.GetParameters()[0].ParameterType;
                var val = Enum.Parse(outType, name);
                return Convert.ToSingle(_brainOutput.Invoke(brain, new[] { val }), CultureInfo.InvariantCulture);
            }
            catch { return float.NaN; }
        }

        private static int IDv(object v) { try { return Convert.ToInt32(v, CultureInfo.InvariantCulture); } catch { return 0; } }
        private static double D(object v) { try { return Convert.ToDouble(v, CultureInfo.InvariantCulture); } catch { return 0.0; } }

        private sealed class TracePre
        {
            public Vector2 rbPos;
            public float rbRot;
            public Vector3 trPos;
            public float trRot;
            public Vector2 rbVel;
            public float rbAngVel;
        }
        private static readonly Dictionary<int, TracePre> TraceBefore = new Dictionary<int, TracePre>();
        private static readonly Dictionary<int, int> SolidContactCount = new Dictionary<int, int>();
        private static readonly Dictionary<int, double> SolidContactImpulse = new Dictionary<int, double>();
        private static readonly Dictionary<string, int> LastContactTick = new Dictionary<string, int>();

        public static void TraceBodyPrefix(object __instance)
        {
            if (_trace == null) return;
            try
            {
                var comp = __instance as Component;
                if (comp == null) return;
                int key = comp.GetInstanceID();
                var pre = new TracePre { trPos = comp.transform.position, trRot = comp.transform.eulerAngles.z };
                var rb = Inst(__instance, "rb2d");
                if (rb != null)
                {
                    try { pre.rbPos = (Vector2)rb.GetType().GetProperty("position").GetValue(rb); } catch { }
                    try { pre.rbRot = Convert.ToSingle(rb.GetType().GetProperty("rotation").GetValue(rb), CultureInfo.InvariantCulture); } catch { }
                    try { pre.rbVel = (Vector2)rb.GetType().GetProperty("linearVelocity").GetValue(rb); } catch { }
                    try { pre.rbAngVel = Convert.ToSingle(rb.GetType().GetProperty("angularVelocity").GetValue(rb), CultureInfo.InvariantCulture); } catch { }
                }
                else pre.rbPos = pre.trPos;
                TraceBefore[key] = pre;
            }
            catch { }
        }

        private static void ContactSummary(Component body, object rb, out int count, out double impulse, out double relVel, out int collidersTouched)
        {
            count = 0; impulse = 0; relVel = 0; collidersTouched = 0;
            try
            {
                var cpType = typeof(ContactPoint2D);
                var colType = typeof(Collider2D);
                if (cpType == null || colType == null || body == null) return;
                foreach (var col in body.GetComponents(colType))
                {
                    if (col == null) continue;
                    var arr = Array.CreateInstance(cpType, 32);
                    var getter = col.GetType().GetMethod("GetContacts", new[] { arr.GetType() });
                    if (getter == null) continue;
                    int n = Convert.ToInt32(getter.Invoke(col, new object[] { arr }), CultureInfo.InvariantCulture);
                    count += n;
                    for (int i = 0; i < n; i++)
                    {
                        object cp = arr.GetValue(i);
                        if (cp == null) continue;
                        collidersTouched++;
                        object imp = null, rel = null;
                        try { imp = cpType.GetField("normalImpulse")?.GetValue(cp) ?? cpType.GetProperty("normalImpulse")?.GetValue(cp); } catch { }
                        try { rel = cpType.GetProperty("relativeVelocity")?.GetValue(cp) ?? cpType.GetField("relativeVelocity")?.GetValue(cp); } catch { }
                        try { impulse += Convert.ToDouble(imp, CultureInfo.InvariantCulture); } catch { }
                        try { relVel += Convert.ToDouble(rel, CultureInfo.InvariantCulture); } catch { }
                    }
                }
            }
            catch { }
        }

        public static void PartCollisionPostfix(object __instance, Collision2D __0)
        {
            try
            {
                _collisionCalls++;
                _collisionContacts += (__0 == null ? 0 : __0.contactCount);
                if (_trace == null) return;
                var bodyType = Plugin.FindType("SimulationScripts.BibiteScripts.BibiteBody");
                var part = __instance as Component;
                if (bodyType == null || part == null) return;
                var parent = part.GetComponentInParent(bodyType) as Component;
                if (parent == null) return;
                int key = parent.GetInstanceID();
                int n = __0 == null ? 0 : __0.contactCount;
                if (n <= 0) return;
                double impulse = 0.0;
                for (int i = 0; i < n; i++)
                {
                    try { var cp = __0.GetContact(i); impulse += Math.Abs(cp.normalImpulse); } catch { }
                }
                int oldN;
                SolidContactCount.TryGetValue(key, out oldN);
                SolidContactCount[key] = oldN + n;
                double oldI;
                SolidContactImpulse.TryGetValue(key, out oldI);
                SolidContactImpulse[key] = oldI + impulse;
            }
            catch { }
        }

        public static void TraceBodyPostfix(object __instance)
        {
            if (_trace == null) return;
            _traceCalls++;
            try
            {
                var body = __instance;
                var comp = body as Component;
                if (comp == null) return;
                object idObj = Inst(body, "id");
                object idNum = Inst(idObj, "id");
                int id = idNum == null ? 0 : IDv(idNum);
                var tr = comp.transform;
                var pos = tr.position;
                float rot = tr.eulerAngles.z;
                var rb = Inst(body, "rb2d");
                double vx = 0, vy = 0, av = 0;
                if (rb != null)
                {
                    try { var v = rb.GetType().GetProperty("linearVelocity").GetValue(rb); vx = D(((Vector2)v).x); vy = D(((Vector2)v).y); } catch { }
                    try { av = D(rb.GetType().GetProperty("angularVelocity").GetValue(rb)); } catch { }
                }
                object eObj = Inst(body, "Energy"); object hObj = Inst(body, "Health");
                double energy = D(Inst(eObj, "Amount"));
                double health = D(Inst(hObj, "Amount"));
                var mouth = Inst(body, "mouth");
                int nInMouth = NMouth(mouth);
                EnsurePlants();
                double nearest = double.NaN, nangle = double.NaN;
                foreach (var pl in _plantPositions)
                {
                    double dx = pl.x - pos.x, dy = pl.y - pos.y;
                    double d = Math.Sqrt(dx * dx + dy * dy);
                    if (double.IsNaN(nearest) || d < nearest) { nearest = d; nangle = Math.Atan2(dy, dx) * 180.0 / Math.PI; }
                }
                object brain = Inst(body, "brain");
                double acc = BrainValue(brain, "Accelerate");
                double rotOut = BrainValue(brain, "Rotate");
                var sb = new StringBuilder(300);
                sb.Append('{');
                sb.Append("\"tick\":").Append(SimTick()).Append(',');
                sb.Append("\"t\":").Append(Plugin.GetSimulatedTime().ToString("0.######", CultureInfo.InvariantCulture)).Append(',');
                sb.Append("\"id\":").Append(id).Append(',');
                sb.Append("\"x\":").Append(pos.x.ToString("0.######", CultureInfo.InvariantCulture)).Append(',');
                sb.Append("\"y\":").Append(pos.y.ToString("0.######", CultureInfo.InvariantCulture)).Append(',');
                sb.Append("\"rot\":").Append(rot.ToString("0.######", CultureInfo.InvariantCulture)).Append(',');
                sb.Append("\"vx\":").Append(vx.ToString("0.######", CultureInfo.InvariantCulture)).Append(',');
                sb.Append("\"vy\":").Append(vy.ToString("0.######", CultureInfo.InvariantCulture)).Append(',');
                sb.Append("\"angvel\":").Append(av.ToString("0.######", CultureInfo.InvariantCulture)).Append(',');
                sb.Append("\"energy\":").Append(energy.ToString("0.######", CultureInfo.InvariantCulture)).Append(',');
                sb.Append("\"health\":").Append(health.ToString("0.######", CultureInfo.InvariantCulture)).Append(',');
                sb.Append("\"n_in_mouth\":").Append(nInMouth).Append(',');
                sb.Append("\"nearest_plant\":").Append(double.IsNaN(nearest) ? "null" : nearest.ToString("0.######", CultureInfo.InvariantCulture)).Append(',');
                sb.Append("\"nearest_plant_angle\":").Append(double.IsNaN(nangle) ? "null" : nangle.ToString("0.######", CultureInfo.InvariantCulture)).Append(',');
                sb.Append("\"accelerate\":").Append(double.IsNaN(acc) ? "null" : acc.ToString("0.######", CultureInfo.InvariantCulture)).Append(',');
                sb.Append("\"rotate\":").Append(double.IsNaN(rotOut) ? "null" : rotOut.ToString("0.######", CultureInfo.InvariantCulture));
                int key2 = comp.GetInstanceID();
                TracePre pre2 = null;
                TraceBefore.TryGetValue(key2, out pre2);
                TraceBefore.Remove(key2);
                if (pre2 != null)
                {
                    sb.Append(",\"pre_x\":").Append(pre2.trPos.x.ToString("0.######", CultureInfo.InvariantCulture));
                    sb.Append(",\"pre_y\":").Append(pre2.trPos.y.ToString("0.######", CultureInfo.InvariantCulture));
                    sb.Append(",\"pre_rot\":").Append(pre2.trRot.ToString("0.######", CultureInfo.InvariantCulture));
                    sb.Append(",\"rb_pre_x\":").Append(pre2.rbPos.x.ToString("0.######", CultureInfo.InvariantCulture));
                    sb.Append(",\"rb_pre_y\":").Append(pre2.rbPos.y.ToString("0.######", CultureInfo.InvariantCulture));
                    sb.Append(",\"rb_pre_rot\":").Append(pre2.rbRot.ToString("0.######", CultureInfo.InvariantCulture));
                    sb.Append(",\"pre_vx\":").Append(pre2.rbVel.x.ToString("0.######", CultureInfo.InvariantCulture));
                    sb.Append(",\"pre_vy\":").Append(pre2.rbVel.y.ToString("0.######", CultureInfo.InvariantCulture));
                    sb.Append(",\"pre_angvel\":").Append(pre2.rbAngVel.ToString("0.######", CultureInfo.InvariantCulture));
                }
                if (rb != null)
                {
                    try { var p = (Vector2)rb.GetType().GetProperty("position").GetValue(rb); sb.Append(",\"rb_x\":").Append(p.x.ToString("0.######", CultureInfo.InvariantCulture)).Append(",\"rb_y\":").Append(p.y.ToString("0.######", CultureInfo.InvariantCulture)); } catch { }
                    try { var rr = Convert.ToSingle(rb.GetType().GetProperty("rotation").GetValue(rb), CultureInfo.InvariantCulture); sb.Append(",\"rb_rot\":").Append(rr.ToString("0.######", CultureInfo.InvariantCulture)); } catch { }
                }
                int cc = 0; double imp = 0.0;
                SolidContactCount.TryGetValue(key2, out cc);
                SolidContactImpulse.TryGetValue(key2, out imp);
                SolidContactCount.Remove(key2);
                SolidContactImpulse.Remove(key2);
                double pImp; int pCount;
                QueryPersistent(body, comp, out pImp, out pCount);
                sb.Append(",\"contact_enter_count\":").Append(cc);
                sb.Append(",\"contact_enter_impulse\":").Append(imp.ToString("0.######", CultureInfo.InvariantCulture));
                sb.Append(",\"persistent_contact_count\":").Append(pCount);
                sb.Append(",\"persistent_impulse\":").Append(pImp.ToString("0.######", CultureInfo.InvariantCulture));
                var mouth2 = Inst(body, "mouth");
                sb.Append(",\"mouth_targets\":").Append(NMouth(mouth2));
                sb.Append('}');
                _trace.WriteLine(sb.ToString());
                _traceRows++;
            }
            catch (Exception e)
            {
                if (_traceErrors == null)
                {
                    _traceErrors = e.ToString();
                    try { File.AppendAllText("Z:\\RUSTibites\\testdata\\parity\\default\\trace_errors.txt", _traceErrors + "\n"); } catch { }
                }
            }
        }

    }

    // =================================================================================================
    // Minimal JSON model + writer (no external dependency; invariant culture, round-trip floats)
    // =================================================================================================
    internal sealed class JObj : List<KeyValuePair<string, object>>
    {
        public JObj Add(string key, object value) { Add(new KeyValuePair<string, object>(key, value)); return this; }
    }

    internal sealed class JArr : List<object>
    {
        public new JArr Add(object value) { base.Add(value); return this; }
    }

    internal static class Json
    {
        public static string Write(object value)
        {
            var sb = new StringBuilder(4096);
            WriteValue(sb, value);
            return sb.ToString();
        }

        private static void WriteValue(StringBuilder sb, object v)
        {
            if (v == null) { sb.Append("null"); return; }
            if (v is string s) { WriteString(sb, s); return; }
            if (v is bool b) { sb.Append(b ? "true" : "false"); return; }
            if (v is float f) { WriteFloat(sb, f); return; }
            if (v is double d) { WriteDouble(sb, d); return; }
            if (v is int i) { sb.Append(i.ToString(CultureInfo.InvariantCulture)); return; }
            if (v is long l) { sb.Append(l.ToString(CultureInfo.InvariantCulture)); return; }
            if (v is short sh) { sb.Append(sh.ToString(CultureInfo.InvariantCulture)); return; }
            if (v is byte by) { sb.Append(by.ToString(CultureInfo.InvariantCulture)); return; }
            if (v is Vector2 v2) { WriteValue(sb, new JObj().Add("x", v2.x).Add("y", v2.y)); return; }
            if (v is Vector3 v3) { WriteValue(sb, new JObj().Add("x", v3.x).Add("y", v3.y).Add("z", v3.z)); return; }
            if (v is Vector4 v4) { WriteValue(sb, new JObj().Add("x", v4.x).Add("y", v4.y).Add("z", v4.z).Add("w", v4.w)); return; }
            if (v is Color col) { WriteValue(sb, new JObj().Add("r", col.r).Add("g", col.g).Add("b", col.b).Add("a", col.a)); return; }
            if (v is Rect rect) { WriteValue(sb, new JObj().Add("x", rect.x).Add("y", rect.y).Add("w", rect.width).Add("h", rect.height)); return; }
            if (v is Bounds bounds) { WriteValue(sb, new JObj().Add("center", bounds.center).Add("size", bounds.size)); return; }
            if (v is Quaternion q) { WriteValue(sb, new JObj().Add("x", q.x).Add("y", q.y).Add("z", q.z).Add("w", q.w)); return; }
            if (v is JObj o)
            {
                sb.Append('{');
                bool first = true;
                foreach (var kv in o)
                {
                    if (!first) sb.Append(',');
                    first = false;
                    WriteString(sb, kv.Key);
                    sb.Append(':');
                    WriteValue(sb, kv.Value);
                }
                sb.Append('}');
                return;
            }
            if (v is JArr a)
            {
                sb.Append('[');
                for (int k = 0; k < a.Count; k++) { if (k > 0) sb.Append(','); WriteValue(sb, a[k]); }
                sb.Append(']');
                return;
            }
            if (v is IList list)
            {
                sb.Append('[');
                for (int k = 0; k < list.Count; k++) { if (k > 0) sb.Append(','); WriteValue(sb, list[k]); }
                sb.Append(']');
                return;
            }
            WriteString(sb, Convert.ToString(v, CultureInfo.InvariantCulture));
        }

        private static void WriteFloat(StringBuilder sb, float f)
        {
            if (float.IsNaN(f)) { sb.Append("\"NaN\""); return; }
            if (float.IsPositiveInfinity(f)) { sb.Append("\"Infinity\""); return; }
            if (float.IsNegativeInfinity(f)) { sb.Append("\"-Infinity\""); return; }
            sb.Append(f.ToString("R", CultureInfo.InvariantCulture));
        }

        private static void WriteDouble(StringBuilder sb, double d)
        {
            if (double.IsNaN(d)) { sb.Append("\"NaN\""); return; }
            if (double.IsPositiveInfinity(d)) { sb.Append("\"Infinity\""); return; }
            if (double.IsNegativeInfinity(d)) { sb.Append("\"-Infinity\""); return; }
            sb.Append(d.ToString("R", CultureInfo.InvariantCulture));
        }

        private static void WriteString(StringBuilder sb, string s)
        {
            sb.Append('"');
            foreach (var ch in s)
            {
                switch (ch)
                {
                    case '"': sb.Append("\\\""); break;
                    case '\\': sb.Append("\\\\"); break;
                    case '\n': sb.Append("\\n"); break;
                    case '\r': sb.Append("\\r"); break;
                    case '\t': sb.Append("\\t"); break;
                    default:
                        if (ch < 32) sb.Append("\\u").Append(((int)ch).ToString("x4", CultureInfo.InvariantCulture));
                        else sb.Append(ch);
                        break;
                }
            }
            sb.Append('"');
        }
    }

    /// <summary>Just enough JSON parsing for the tiny request bodies ({"lock": true}).</summary>
    internal static class MiniJson
    {
        public static Dictionary<string, object> ParseObject(string text)
        {
            var map = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
            if (string.IsNullOrWhiteSpace(text)) return map;
            int i = 0;
            SkipWs(text, ref i);
            if (i >= text.Length || text[i] != '{') return map;
            i++;
            while (i < text.Length)
            {
                SkipWs(text, ref i);
                if (i >= text.Length || text[i] == '}') break;
                string key = ReadString(text, ref i);
                SkipWs(text, ref i);
                if (i < text.Length && text[i] == ':') i++;
                SkipWs(text, ref i);
                object val = ReadValue(text, ref i);
                if (key != null) map[key] = val;
                SkipWs(text, ref i);
                if (i < text.Length && text[i] == ',') { i++; continue; }
                break;
            }
            return map;
        }

        public static bool Truthy(object v)
        {
            if (v == null) return false;
            if (v is bool b) return b;
            if (v is string s) return s.Equals("true", StringComparison.OrdinalIgnoreCase) || s == "1";
            if (v is double d) return d != 0;
            return false;
        }

        private static void SkipWs(string t, ref int i) { while (i < t.Length && char.IsWhiteSpace(t[i])) i++; }

        private static string ReadString(string t, ref int i)
        {
            if (i >= t.Length || t[i] != '"') return null;
            i++;
            var sb = new StringBuilder();
            while (i < t.Length && t[i] != '"')
            {
                if (t[i] == '\\' && i + 1 < t.Length)
                {
                    i++;
                    char c = t[i];
                    if (c == 'n') sb.Append('\n');
                    else if (c == 'r') sb.Append('\r');
                    else if (c == 't') sb.Append('\t');
                    else if (c == 'u' && i + 4 < t.Length) { sb.Append((char)Convert.ToInt32(t.Substring(i + 1, 4), 16)); i += 4; }
                    else sb.Append(c);
                }
                else sb.Append(t[i]);
                i++;
            }
            if (i < t.Length) i++;
            return sb.ToString();
        }

        private static object ReadValue(string t, ref int i)
        {
            if (i >= t.Length) return null;
            char c = t[i];
            if (c == '"') return ReadString(t, ref i);
            if (c == '[')
            {
                var list = new List<object>();
                i++;
                while (i < t.Length)
                {
                    SkipWs(t, ref i);
                    if (i >= t.Length) break;
                    if (t[i] == ']') { i++; break; }
                    list.Add(ReadValue(t, ref i));
                    SkipWs(t, ref i);
                    if (i < t.Length && t[i] == ',') i++;
                }
                return list;
            }
            int start = i;
            while (i < t.Length && ",}] \t\r\n".IndexOf(t[i]) < 0) i++;
            string token = t.Substring(start, i - start).Trim();
            if (token.Equals("true", StringComparison.OrdinalIgnoreCase)) return true;
            if (token.Equals("false", StringComparison.OrdinalIgnoreCase)) return false;
            double d;
            if (double.TryParse(token, NumberStyles.Float, CultureInfo.InvariantCulture, out d)) return d;
            return token;
        }
    }
}





