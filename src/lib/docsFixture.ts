const now = Math.floor(Date.now() / 1000);

const macros = [
  {
    name: "Daily Quest Route",
    events: 184,
    duration: 148,
    resolution: "1920x1080",
    loop: true,
    loop_count: 3,
    category: "Daily",
    notes: "Collect quests, complete the route, and claim the daily reward.",
    play_count: 42,
    last_played: now - 18 * 60,
    played: 6216,
  },
  {
    name: "Dungeon Queue",
    events: 96,
    duration: 82,
    resolution: "1920x1080",
    loop: false,
    loop_count: 1,
    category: "Dungeons",
    notes: "Queue, confirm the party, and enter the dungeon.",
    play_count: 28,
    last_played: now - 3 * 60 * 60,
    played: 2296,
  },
  {
    name: "Reward Claim",
    events: 38,
    duration: 24,
    resolution: "1920x1080",
    loop: false,
    loop_count: 1,
    category: "Daily",
    notes: "Open the reward screen and collect available items.",
    play_count: 21,
    last_played: now - 26 * 60 * 60,
    played: 504,
  },
  {
    name: "Reconnect Recovery",
    events: 61,
    duration: 47,
    resolution: "1920x1080",
    loop: false,
    loop_count: 1,
    category: "Recovery",
    notes: "Reconnect, wait for the lobby, and restore the previous route.",
    play_count: 17,
    last_played: now - 2 * 24 * 60 * 60,
    played: 799,
  },
  {
    name: "Merchant Restock",
    events: 73,
    duration: 66,
    resolution: "1920x1080",
    loop: true,
    loop_count: 5,
    category: "Farming",
    notes: "Check the merchant inventory and purchase configured items.",
    play_count: 14,
    last_played: now - 4 * 24 * 60 * 60,
    played: 924,
  },
  {
    name: "Inventory Cleanup",
    events: 112,
    duration: 91,
    resolution: "1920x1080",
    loop: false,
    loop_count: 1,
    category: "Utility",
    notes: "Sort inventory and clear common items.",
    play_count: 11,
    last_played: now - 6 * 24 * 60 * 60,
    played: 1001,
  },
  {
    name: "Boss Rotation",
    events: 248,
    duration: 214,
    resolution: "1920x1080",
    loop: true,
    loop_count: 0,
    category: "Combat",
    notes: "Repeat the configured boss rotation until stopped.",
    play_count: 9,
    last_played: now - 8 * 24 * 60 * 60,
    played: 1926,
  },
  {
    name: "Event Farm",
    events: 156,
    duration: 129,
    resolution: "1920x1080",
    loop: true,
    loop_count: 10,
    category: "Farming",
    notes: "Run the current event route and collect the completion reward.",
    play_count: 7,
    last_played: now - 10 * 24 * 60 * 60,
    played: 903,
  },
];

const emptyStep = (id: string, type: string, label: string) => ({
  id,
  type,
  enabled: true,
  label,
  x: 0,
  y: 0,
  key: "",
  text: "",
  delay: type === "delay" ? 2 : 0,
  scroll_amount: 0,
  detect_mode: type === "wait_for" ? "template" : "color",
  hsv_low: [0, 0, 0],
  hsv_high: [179, 255, 255],
  template: type === "wait_for" ? "reward-button.png" : "",
  region: [60, 45, 100, 100],
  min_area: 40,
  timeout: 12,
  confidence: 0.88,
});

const rewardPreviewSvg = `
<svg xmlns="http://www.w3.org/2000/svg" width="720" height="320" viewBox="0 0 720 320">
  <rect width="720" height="320" fill="#17191f"/>
  <rect x="28" y="28" width="664" height="264" rx="18" fill="#242832" stroke="#454b59"/>
  <text x="64" y="88" fill="#f5f7fb" font-family="Segoe UI,Arial,sans-serif" font-size="25" font-weight="600">Daily quest complete</text>
  <text x="64" y="124" fill="#aeb6c5" font-family="Segoe UI,Arial,sans-serif" font-size="16">Your reward is ready to collect.</text>
  <rect x="432" y="202" width="220" height="58" rx="11" fill="#d69b2d"/>
  <text x="542" y="238" text-anchor="middle" fill="#181512" font-family="Segoe UI,Arial,sans-serif" font-size="17" font-weight="700">CLAIM REWARD</text>
  <rect x="420" y="190" width="244" height="82" rx="15" fill="none" stroke="#f0c66d" stroke-width="3"/>
  <circle cx="542" cy="231" r="7" fill="none" stroke="#ffffff" stroke-width="2"/>
  <path d="M542 216v8M542 238v8M527 231h8M549 231h8" stroke="#ffffff" stroke-width="2"/>
</svg>`.trim();

const rewardPreviewBase64 = btoa(rewardPreviewSvg);
const rewardPreviewDataUri =
  `data:image/svg+xml;charset=utf-8,${encodeURIComponent(rewardPreviewSvg)}`;

const embeddedMacroSteps = [
  { ...emptyStep("macro-1", "click", "Open quest list"), x: 1560, y: 88 },
  { ...emptyStep("macro-2", "key", "Confirm quest"), key: "enter" },
  { ...emptyStep("macro-3", "delay", "Wait for transition"), delay: 1.5 },
  { ...emptyStep("macro-4", "click", "Start route"), x: 1470, y: 930 },
];

const graph = {
  version: 1,
  name: "Daily Quest Rotation",
  entry: "start",
  nodes: [
    {
      id: "start",
      type: "start",
      label: "Start",
      position: { x: 0, y: 250 },
      enabled: true,
      config: {},
    },
    {
      id: "daily-route",
      type: "sub_macro",
      label: "Daily Quest Route",
      position: { x: 205, y: 250 },
      enabled: true,
      config: {
        macro_name: "Daily Quest Route",
        embedded_steps: embeddedMacroSteps,
        repeat: 1,
        source_events: 184,
        source_duration: 148,
        source_resolution: "1920x1080",
      },
    },
    {
      id: "reward-check",
      type: "vision",
      label: "Wait for reward",
      position: { x: 430, y: 115 },
      enabled: true,
      config: {
        step: emptyStep("vision-1", "wait_for", "Wait for reward"),
        template_thumb: rewardPreviewDataUri,
      },
    },
    {
      id: "result",
      type: "branch",
      label: "Reward visible?",
      position: { x: 655, y: 115 },
      enabled: true,
      config: { condition: "last_ok" },
    },
    {
      id: "recovery",
      type: "action",
      label: "Recovery",
      position: { x: 660, y: 385 },
      enabled: true,
      config: {
        step: { ...emptyStep("recover-1", "key", "Open recovery menu"), key: "escape" },
      },
    },
    {
      id: "delay",
      type: "action",
      label: "Wait before next run",
      position: { x: 890, y: 175 },
      enabled: true,
      config: {
        step: { ...emptyStep("delay-1", "delay", "Wait before next run"), delay: 5 },
      },
    },
    {
      id: "complete",
      type: "stop",
      label: "Complete",
      position: { x: 1115, y: 175 },
      enabled: true,
      config: { success: true },
    },
    {
      id: "failed",
      type: "stop",
      label: "Stop safely",
      position: { x: 890, y: 430 },
      enabled: true,
      config: { success: false },
    },
  ],
  edges: [
    { id: "e-start-route", from: "start", output: "next", to: "daily-route" },
    { id: "e-route-check", from: "daily-route", output: "success", to: "reward-check" },
    { id: "e-route-recovery", from: "daily-route", output: "error", to: "recovery" },
    { id: "e-check-branch", from: "reward-check", output: "found", to: "result" },
    { id: "e-check-recovery", from: "reward-check", output: "missing", to: "recovery" },
    { id: "e-works-delay", from: "result", output: "true", to: "delay" },
    { id: "e-fails-recovery", from: "result", output: "false", to: "recovery" },
    { id: "e-recovery-delay", from: "recovery", output: "next", to: "delay" },
    { id: "e-recovery-stop", from: "recovery", output: "error", to: "failed" },
    { id: "e-delay-stop", from: "delay", output: "next", to: "complete" },
    { id: "e-delay-fail", from: "delay", output: "error", to: "failed" },
  ],
};

const rewardTrigger = {
  id: "reward-available",
  name: "Reward available",
  method: "template",
  action: "click",
  key: "",
  hsv_low: [0, 0, 0],
  hsv_high: [179, 255, 255],
  template_path: "reward-button.png",
  ocr_text: "",
  threshold: 0.88,
  region: [60, 45, 100, 100],
  min_area: 40,
  resume_delay: 2,
  cooldown: 5,
  enabled: true,
  click_offset: [110, 29],
  click_line: [],
  click_lines: [],
};

/**
 * Deterministic, read-only data for repository screenshots. `api.ts` only
 * reaches this module from a Vite development build with `?docs=1`.
 */
export async function docsInvoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const ok = { ok: true };
  switch (command) {
    case "get_status":
      return {
        mode: "idle",
        elapsed: 0,
        record_paused: false,
        play_iteration: 0,
        play_total_reps: 0,
        window: { found: true, title: "Roblox", size: "1920x1080" },
        capture: { backend: "DXGI", fps: 60 },
        recorded_count: 0,
        last_macro: "Daily Quest Route",
        macro_count: macros.length,
        indicator_alive: true,
        config: {
          resolution: [1920, 1080],
          capture_backend: "dxgi",
          hotkey_record: "F6",
          hotkey_play: "F7",
          hotkey_stop: "F8",
          indicator_on_top: true,
        },
        log: [],
      } as T;
    case "get_stats_summary":
      return {
        total_plays: 149,
        macros_played: 8,
        most_played: "Daily Quest Route",
        most_played_count: 42,
        total_macros: macros.length,
        total_chains: 3,
        total_guards: 6,
        total_schedules: 2,
      } as T;
    case "get_run_history":
      return [
        { name: "Daily Quest Route", timestamp: now - 18 * 60, duration: 148, status: "completed" },
        { name: "Dungeon Queue", timestamp: now - 3 * 60 * 60, duration: 82, status: "completed" },
        { name: "Reward Claim", timestamp: now - 26 * 60 * 60, duration: 24, status: "completed" },
        { name: "Reconnect Recovery", timestamp: now - 2 * 24 * 60 * 60, duration: 47, status: "stopped" },
      ].slice(0, Number(args?.limit ?? 6)) as T;
    case "anti_afk_list_windows":
      return [
        { id: "roblox", title: "Roblox", pid: 4108 },
        { id: "game-client", title: "Game Client", pid: 5520 },
      ] as T;
    case "anti_afk_get":
      return {
        enabled: false,
        target_id: "roblox",
        interval_min: 12,
        action: "random",
        status: "off",
        error: null,
      } as T;
    case "list_macros":
      return macros as T;
    case "list_templates":
      return [
        { name: "Quest routine", events: 184, duration: 148, category: "Daily" },
        { name: "Recovery flow", events: 61, duration: 47, category: "Recovery" },
      ] as T;
    case "get_all_guard_counts":
      return {
        ok: true,
        counts: {
          "Daily Quest Route": 2,
          "Dungeon Queue": 1,
          "Reconnect Recovery": 2,
          "Boss Rotation": 1,
        },
      } as T;
    case "node_graph_list":
      return [
        { name: "Daily Quest Rotation", nodes: graph.nodes.length, valid_file: true, updated_at: now - 900 },
        { name: "Dungeon Recovery", nodes: 7, valid_file: true, updated_at: now - 86400 },
        { name: "Event Rotation", nodes: 6, valid_file: true, updated_at: now - 172800 },
      ] as T;
    case "node_graph_load":
      return { ok: true, graph, source: "saved" } as T;
    case "node_graph_validate":
      return { ok: true, errors: [], warnings: [] } as T;
    case "list_chains":
      return [
        {
          id: "daily-chain",
          name: "Daily rotation",
          macro_names: ["Daily Quest Route", "Reward Claim"],
          delay_between: 3,
          repeat: 1,
        },
      ] as T;
    case "macro_to_steps":
      return { ok: true, steps: embeddedMacroSteps, count: embeddedMacroSteps.length } as T;
    case "vision_load":
      return { ok: true, triggers: [rewardTrigger] } as T;
    case "vision_status":
      return {
        ok: true,
        running: false,
        fired: 12,
        log: [
          { kind: "act", msg: "'Reward available' -> clicked (1542, 884)" },
          { kind: "match", msg: "Reward available matched at 91%" },
        ],
      } as T;
    case "guard_test":
      return {
        ok: true,
        matched: 1,
        found_x: 1542,
        found_y: 884,
        confidence: 0.91,
        message: "Reward button found.",
        preview: rewardPreviewBase64,
        preview_mime: "image/svg+xml",
      } as T;
    case "get_config":
      return {
        capture_backend: "dxgi",
        hotkey_record: "F6",
        hotkey_play: "F7",
        hotkey_stop: "F8",
        indicator_on_top: true,
        humanize_clicks: true,
        notify_on_schedule: false,
        notify_on_complete: false,
      } as T;
    case "get_data_paths":
      return {
        ok: true,
        root: "Clawmation",
        macros_dir: "macros",
        templates_dir: "templates",
        snapshots_dir: "snapshots",
        config_dir: "config",
        macro_count: macros.length,
        template_count: 2,
        snapshot_count: 4,
      } as T;
    case "get_version":
      return { version: "1.2.1" } as T;
    case "check_for_updates":
      return { ok: true, current: "1.2.1", latest: "1.2.1", available: false } as T;
    case "node_graph_create":
      return { ok: true, name: "New Loop", graph } as T;
    case "node_graph_rename":
      return { ok: true, name: String(args?.newName ?? graph.name), graph } as T;
    case "stop_record":
      return {
        ok: true,
        name: "New recording",
        events: 32,
        duration: 18,
        resolution: "1920x1080",
      } as T;
    case "anti_afk_update":
      return { ok: true, state: await docsInvoke("anti_afk_get") } as T;
    default:
      return ok as T;
  }
}
