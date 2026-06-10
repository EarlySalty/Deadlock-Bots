const fs = require("node:fs");
const path = require("node:path");
const { DatabaseSync } = require("node:sqlite");

const protobuf = require("protobufjs");
const { createCustomLobbyHandlers } = require("./src/custom_lobby");

const PROTO_MASK = 0x80000000 >>> 0;
const DEADLOCK_APP_ID = Number.parseInt(process.env.DEADLOCK_APP_ID || "1422450", 10);
const DB_PATH =
  process.env.DEADLOCK_DB_PATH ||
  path.join(process.env.USERPROFILE || process.cwd(), "Documents", "Deadlock", "service", "deadlock.sqlite3");
const POLL_INTERVAL_MS = Number.parseInt(process.env.STEAM_TASK_POLL_MS || "2000", 10);
const HEARTBEAT_INTERVAL_MS = Number.parseInt(process.env.STEAM_HEARTBEAT_MS || "5000", 10);
const TASK_DELAY_MS = Number.parseInt(process.env.STEAM_TASK_DELAY_MS || "500", 10);
const DEFAULT_TIMEOUT_MS = Number.parseInt(process.env.STEAM_GC_RESPONSE_TIMEOUT_MS || "20000", 10);
const REFRESH_TOKEN_PATH =
  process.env.STEAM_REFRESH_TOKEN_PATH ||
  path.join(process.env.STEAM_PRESENCE_DATA_DIR || path.join(__dirname, ".steam-data"), "refresh.token");
const GC_MSG_CLIENT_HELLO = 4006;
const GC_MSG_CLIENT_WELCOME = 4004;
const GC_MSG_SO_SINGLE_OBJECT = 21;
const GC_MSG_SO_MULTIPLE_OBJECTS = 26;
const GC_MSG_SO_CACHE_SUBSCRIBED = 24;
const GC_MSG_SO_CACHE_SUBSCRIBED_UP_TO_DATE = 29;
const SUPPORTED_TASK_TYPES = [
  "GC_CREATE_CUSTOM_LOBBY",
  "GC_LOBBY_SET_SPECTATOR",
  "GC_LOBBY_READY",
  "GC_LOBBY_START_MATCH",
  "GC_LOBBY_LEAVE",
  "GC_GET_MATCH_RESULT",
];

const GC = {
  PARTY_CREATE: 9123,
  PARTY_CREATE_RESPONSE: 9124,
  PARTY_LEAVE: 9125,
  PARTY_LEAVE_RESPONSE: 9126,
  PARTY_ACTION: 9129,
  PARTY_ACTION_RESPONSE: 9130,
  PARTY_START_MATCH: 9131,
  PARTY_START_MATCH_RESPONSE: 9132,
  PARTY_READY: 9142,
  PARTY_READY_RESPONSE: 9143,
  GET_MATCH_METADATA: 9167,
  GET_MATCH_METADATA_RESPONSE: 9168,
  GET_ACTIVE_MATCHES: 9203,
  GET_ACTIVE_MATCHES_RESPONSE: 9204,
};

function nowMs() {
  return Date.now();
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function log(level, message, extra) {
  const stamp = new Date().toISOString();
  const suffix = extra ? ` ${JSON.stringify(extra)}` : "";
  console[level](`[SteamBridge] ${stamp} ${message}${suffix}`);
}

function formatPartyCode(code) {
  const digits = String(code || "").replace(/\D/g, "");
  if (digits.length !== 9) {
    return digits || null;
  }
  return `${digits.slice(0, 3)}-${digits.slice(3, 6)}-${digits.slice(6, 9)}`;
}

function asUnsigned(value) {
  return value >>> 0;
}

function stripProtoMask(msgType) {
  return asUnsigned(msgType) & ~PROTO_MASK;
}

function toJsonSafe(value) {
  if (typeof value === "bigint") {
    return value.toString();
  }
  if (!value || typeof value !== "object") {
    return value;
  }
  if (typeof value.toNumber === "function" && typeof value.toString === "function") {
    const asNumber = value.toNumber();
    if (Number.isSafeInteger(asNumber)) {
      return asNumber;
    }
    return value.toString();
  }
  if (Buffer.isBuffer(value) || value instanceof Uint8Array) {
    return Buffer.from(value).toString("base64");
  }
  if (Array.isArray(value)) {
    return value.map((entry) => toJsonSafe(entry));
  }
  const result = {};
  for (const [key, entry] of Object.entries(value)) {
    result[key] = toJsonSafe(entry);
  }
  return result;
}

class ProtoBundle {
  constructor() {
    this.baseDir = path.join(__dirname, "protos");
    this.root = new protobuf.Root();
    this.root.resolvePath = (origin, target) => {
      const normalized = target.replace(/\\/g, "/");
      const candidateLocal = path.join(this.baseDir, normalized);
      if (fs.existsSync(candidateLocal)) {
        return candidateLocal;
      }
      const deadlockCandidate = path.join(this.baseDir, "deadlock", normalized);
      if (fs.existsSync(deadlockCandidate)) {
        return deadlockCandidate;
      }
      return protobuf.util.path.resolve(origin, target);
    };
  }

  async load() {
    const entryFile = path.join(this.baseDir, "deadlock", "citadel_gcmessages_client.proto");
    await this.root.load(entryFile, { keepCase: true });
    this.root.resolveAll();
  }

  encode(typeName, payload = {}) {
    const type = this.root.lookupType(typeName);
    return type.encode(type.fromObject(payload)).finish();
  }

  decode(typeName, buffer) {
    const type = this.root.lookupType(typeName);
    return toJsonSafe(type.toObject(type.decode(buffer), {
      longs: String,
      enums: String,
      bytes: Buffer,
      defaults: false,
    }));
  }

  tryDecode(typeName, buffer) {
    try {
      return this.decode(typeName, buffer);
    } catch {
      return null;
    }
  }
}

class TaskStore {
  constructor(dbPath) {
    this.db = new DatabaseSync(dbPath);
    this.db.exec("PRAGMA busy_timeout = 5000;");
    this.db.exec("PRAGMA journal_mode = WAL;");
  }

  claimNextTask(taskTypes) {
    const placeholders = taskTypes.map(() => "?").join(", ");
    const selectStmt = this.db.prepare(
      `
      SELECT id, type, payload, attempts
      FROM steam_tasks
      WHERE status = 'PENDING' AND type IN (${placeholders})
      ORDER BY id ASC
      LIMIT 1
      `
    );
    const row = selectStmt.get(...taskTypes);
    if (!row) {
      return null;
    }

    const now = nowMs();
    const update = this.db.prepare(
      `
      UPDATE steam_tasks
      SET status = 'RUNNING',
          started_at = ?,
          updated_at = ?,
          attempts = COALESCE(attempts, 0) + 1
      WHERE id = ? AND status = 'PENDING'
      `
    ).run(now, now, row.id);

    if (!update.changes) {
      return null;
    }

    let parsedPayload = {};
    if (row.payload) {
      try {
        parsedPayload = JSON.parse(row.payload);
      } catch (error) {
        this.failTask(row.id, `Invalid JSON payload: ${error.message}`);
        return null;
      }
    }

    return {
      id: row.id,
      type: row.type,
      payload: parsedPayload,
      attempts: row.attempts || 0,
    };
  }

  completeTask(taskId, result) {
    const now = nowMs();
    this.db.prepare(
      `
      UPDATE steam_tasks
      SET status = 'DONE',
          result = ?,
          error = NULL,
          updated_at = ?,
          finished_at = ?
      WHERE id = ?
      `
    ).run(JSON.stringify(result), now, now, taskId);
  }

  failTask(taskId, errorMessage) {
    const now = nowMs();
    this.db.prepare(
      `
      UPDATE steam_tasks
      SET status = 'FAILED',
          error = ?,
          updated_at = ?,
          finished_at = ?
      WHERE id = ?
      `
    ).run(String(errorMessage || "Unknown error"), now, now, taskId);
  }

  writeHeartbeat(payload) {
    const heartbeat = nowMs();
    this.db.prepare(
      `
      INSERT INTO standalone_bot_state(bot, heartbeat, payload, updated_at)
      VALUES ('steam', ?, ?, datetime('now'))
      ON CONFLICT(bot) DO UPDATE SET
        heartbeat = excluded.heartbeat,
        payload = excluded.payload,
        updated_at = datetime('now')
      `
    ).run(heartbeat, JSON.stringify(payload));
  }
}

class MockGcAdapter {
  constructor() {
    this.mode = "mock";
    this.partyState = new Map();
    this.matchState = new Map();
    this.lastError = null;
    this.startedAt = nowMs();
  }

  async connect() {
    log("warn", "Steam Bridge laeuft im Mock-Modus. GC-Aktionen werden simuliert.");
  }

  getRuntime() {
    return {
      mode: this.mode,
      logged_on: true,
      deadlock_gc_ready: true,
      last_error: this.lastError,
      parties_cached: this.partyState.size,
      matches_cached: this.matchState.size,
      started_at: this.startedAt,
    };
  }

  async handleTask(type, payload) {
    switch (type) {
      case "GC_CREATE_CUSTOM_LOBBY":
        return this.createLobby(payload);
      case "GC_LOBBY_SET_SPECTATOR":
      case "GC_LOBBY_READY":
      case "GC_LOBBY_LEAVE":
        return { success: true };
      case "GC_LOBBY_START_MATCH":
        return this.startMatch(payload);
      case "GC_GET_MATCH_RESULT":
        return this.getMatchResult(payload);
      default:
        throw new Error(`Unsupported task type ${type}`);
    }
  }

  createLobby() {
    const partyId = `${nowMs()}${Math.floor(Math.random() * 1000)}`;
    const joinCode = `${Math.floor(100000000 + Math.random() * 900000000)}`;
    this.partyState.set(partyId, { joinCode, matchId: null });
    return {
      success: true,
      party_id: partyId,
      join_code: joinCode,
      party_code_display: formatPartyCode(joinCode),
    };
  }

  startMatch(payload) {
    const partyId = String(payload.party_id);
    if (!this.partyState.has(partyId)) {
      throw new Error(`Party ${partyId} not found`);
    }
    const matchId = Number(`${Date.now()}`.slice(-10));
    this.partyState.get(partyId).matchId = matchId;
    this.matchState.set(String(matchId), {
      winning_team: matchId % 2,
      duration_s: 1800 + (matchId % 600),
      players: [],
    });
    return {
      success: true,
      match_id: matchId,
    };
  }

  getMatchResult(payload) {
    const explicitMatchId = payload.match_id ? String(payload.match_id) : null;
    const partyId = payload.party_id ? String(payload.party_id) : null;
    const resolvedMatchId =
      explicitMatchId ||
      (partyId && this.partyState.has(partyId) ? String(this.partyState.get(partyId).matchId) : null);

    if (!resolvedMatchId || !this.matchState.has(resolvedMatchId)) {
      throw new Error("Kein simuliertes Match-Ergebnis vorhanden");
    }

    const match = this.matchState.get(resolvedMatchId);
    return {
      success: true,
      match_id: Number(resolvedMatchId),
      winning_team: match.winning_team,
      duration_s: match.duration_s,
      players: match.players,
    };
  }
}

class SteamGcAdapter {
  constructor(proto) {
    this.proto = proto;
    this.mode = "steam";
    this.client = null;
    this.SteamUser = null;
    this.loggedOn = false;
    this.gcReady = false;
    this.lastError = null;
    this.startedAt = nowMs();
    this.responseWaiters = new Map();
    this.partyCache = new Map();
    this.lobbyCache = new Map();
    this.partyToMatch = new Map();
    this.matchSnapshots = new Map();
    this.customLobby = createCustomLobbyHandlers(this);
    this.gc = GC;
    this.sleep = sleep;
    this.log = log;
    this.formatPartyCode = formatPartyCode;
  }

  async connect() {
    const SteamUserModule = require("steam-user");
    this.SteamUser = SteamUserModule;
    this.client = new SteamUserModule({
      autoRelogin: true,
      renewRefreshTokens: true,
      promptSteamGuardCode: false,
    });

    this.client.on("loggedOn", () => {
      this.loggedOn = true;
      this.lastError = null;
      log("log", "Steam eingeloggt");
      try {
        if (this.SteamUser?.EPersonaState) {
          this.client.setPersona(this.SteamUser.EPersonaState.Online);
        }
      } catch {}
      this.client.gamesPlayed([DEADLOCK_APP_ID]);
      this.sendClientHello().catch((error) => {
        this.lastError = error.message;
        log("error", "GC ClientHello fehlgeschlagen", { error: error.message });
      });
    });

    this.client.on("error", (error) => {
      this.lastError = error.message;
      log("error", "Steam-Fehler", { error: error.message });
    });

    this.client.on("disconnected", (eresult, msg) => {
      this.loggedOn = false;
      this.gcReady = false;
      this.lastError = `Disconnected: ${eresult} ${msg || ""}`.trim();
      log("warn", "Steam getrennt", { eresult, msg });
    });

    this.client.on("refreshToken", (token) => {
      fs.mkdirSync(path.dirname(REFRESH_TOKEN_PATH), { recursive: true });
      fs.writeFileSync(REFRESH_TOKEN_PATH, token, "utf8");
      log("log", "Refresh-Token aktualisiert");
    });

    this.client.on("receivedFromGC", (appid, msgType, payload) => {
      if (Number(appid) !== DEADLOCK_APP_ID) {
        return;
      }
      this.handleGcMessage(msgType, Buffer.from(payload));
    });

    const loginOptions = this.getLoginOptions();
    this.client.logOn(loginOptions);
  }

  getLoginOptions() {
    const options = {};
    const refreshToken =
      process.env.STEAM_REFRESH_TOKEN ||
      (fs.existsSync(REFRESH_TOKEN_PATH) ? fs.readFileSync(REFRESH_TOKEN_PATH, "utf8").trim() : "");

    if (refreshToken) {
      options.refreshToken = refreshToken;
      return options;
    }

    if (!process.env.STEAM_USERNAME || !process.env.STEAM_PASSWORD) {
      throw new Error(
        "Kein Steam-Login konfiguriert. Erforderlich: STEAM_REFRESH_TOKEN oder STEAM_USERNAME/STEAM_PASSWORD."
      );
    }

    options.accountName = process.env.STEAM_USERNAME;
    options.password = process.env.STEAM_PASSWORD;
    if (process.env.STEAM_SHARED_SECRET) {
      options.twoFactorCode = this.SteamUser.generateAuthCode(process.env.STEAM_SHARED_SECRET);
    }
    return options;
  }

  getRuntime() {
    return {
      mode: this.mode,
      logged_on: this.loggedOn,
      deadlock_gc_ready: this.gcReady,
      last_error: this.lastError,
      parties_cached: this.partyCache.size,
      lobbies_cached: this.lobbyCache.size,
      matches_cached: this.matchSnapshots.size,
      started_at: this.startedAt,
    };
  }

  async handleTask(type, payload) {
    if (!this.loggedOn || !this.gcReady) {
      throw new Error("Steam/Deadlock GC ist noch nicht bereit");
    }
    return this.customLobby.handleTask(type, payload);
  }

  async sendClientHello() {
    const payload = this.proto.encode("CMsgClientHello", {
      version: 1,
      engine: "k_ESE_Source2",
      platform_name: process.platform,
      game_msg: this.proto.encode("CMsgCitadelClientHello", {}),
    });
    this.client.sendToGC(DEADLOCK_APP_ID, PROTO_MASK | GC_MSG_CLIENT_HELLO, {}, payload);
  }

  async sendAndWait({ msgId, typeName, payload, responseMsgId, responseTypeName, timeoutMs = DEFAULT_TIMEOUT_MS }) {
    const waiter = {};
    const queue = this.responseWaiters.get(responseMsgId) || [];
    waiter.promise = new Promise((resolve, reject) => {
      waiter.resolve = resolve;
      waiter.reject = reject;
      waiter.timer = setTimeout(() => {
        this.responseWaiters.set(
          responseMsgId,
          (this.responseWaiters.get(responseMsgId) || []).filter((entry) => entry !== waiter)
        );
        reject(new Error(`GC response timeout for ${responseMsgId}`));
      }, timeoutMs);
    });
    waiter.responseTypeName = responseTypeName;
    queue.push(waiter);
    this.responseWaiters.set(responseMsgId, queue);

    const encoded = this.proto.encode(typeName, payload);
    this.client.sendToGC(DEADLOCK_APP_ID, PROTO_MASK | msgId, {}, encoded);

    return waiter.promise;
  }

  async waitForPartyJoinCode(partyId, timeoutMs = DEFAULT_TIMEOUT_MS) {
    const deadline = nowMs() + timeoutMs;
    while (nowMs() < deadline) {
      const party = this.partyCache.get(String(partyId));
      if (party?.join_code) {
        return party;
      }
      await sleep(250);
    }
    throw new Error(`Timeout while waiting for join code for party ${partyId}`);
  }

  async queryActiveMatches() {
    const response = await this.sendAndWait({
      msgId: GC.GET_ACTIVE_MATCHES,
      typeName: "CMsgClientToGCGetActiveMatches",
      payload: {},
      responseMsgId: GC.GET_ACTIVE_MATCHES_RESPONSE,
      responseTypeName: "CMsgClientToGCGetActiveMatchesResponse",
      timeoutMs: 10000,
    });

    const activeMatches = Array.isArray(response.active_matches) ? response.active_matches : [];
    for (const match of activeMatches) {
      const matchId = match.match_id ? String(match.match_id) : null;
      const lobbyId = match.lobby_id ? String(match.lobby_id) : null;
      if (matchId) {
        this.matchSnapshots.set(matchId, {
          winning_team: this.normalizeWinningTeam(match.winning_team),
          duration_s: match.duration_s ?? null,
          players: (match.players || []).map((player) => ({
            account_id: player.account_id,
            team: this.normalizeWinningTeam(player.team),
            hero_id: player.hero_id,
            kills: null,
            deaths: null,
            assists: null,
            net_worth: null,
            last_hits: null,
          })),
        });
      }
      if (lobbyId && matchId) {
        this.partyToMatch.set(lobbyId, matchId);
      }
    }

    return activeMatches;
  }

  handleGcMessage(msgType, payload) {
    const normalizedType = stripProtoMask(msgType);

    if (normalizedType === GC_MSG_CLIENT_WELCOME) {
      this.gcReady = true;
      const welcome = this.proto.tryDecode("CMsgClientWelcome", payload);
      for (const cache of welcome?.outofdate_subscribed_caches || []) {
        this.ingestSubscribedCache(cache);
      }
      return;
    }

    if (normalizedType === GC_MSG_SO_CACHE_SUBSCRIBED) {
      const message = this.proto.tryDecode("CMsgSOCacheSubscribed", payload);
      if (message) {
        this.ingestSubscribedCache(message);
      }
      return;
    }

    if (normalizedType === GC_MSG_SO_SINGLE_OBJECT) {
      const message = this.proto.tryDecode("CMsgSOSingleObject", payload);
      if (message?.object_data) {
        this.ingestCacheObject(Buffer.from(message.object_data, "base64"));
      }
      return;
    }

    if (normalizedType === GC_MSG_SO_MULTIPLE_OBJECTS) {
      const message = this.proto.tryDecode("CMsgSOMultipleObjects", payload);
      for (const entry of message?.objects_added || []) {
        if (entry.object_data) {
          this.ingestCacheObject(Buffer.from(entry.object_data, "base64"));
        }
      }
      for (const entry of message?.objects_modified || []) {
        if (entry.object_data) {
          this.ingestCacheObject(Buffer.from(entry.object_data, "base64"));
        }
      }
      return;
    }

    const waiters = this.responseWaiters.get(normalizedType);
    if (waiters?.length) {
      const waiter = waiters.shift();
      if (!waiters.length) {
        this.responseWaiters.delete(normalizedType);
      }
      clearTimeout(waiter.timer);
      try {
        waiter.resolve(this.proto.decode(waiter.responseTypeName, payload));
      } catch (error) {
        waiter.reject(error);
      }
      return;
    }

    if (normalizedType !== GC_MSG_SO_CACHE_SUBSCRIBED_UP_TO_DATE) {
      log("log", "Unbehandelter GC-Message-Typ empfangen", { msgType: normalizedType });
    }
  }

  ingestSubscribedCache(message) {
    for (const objectGroup of message?.objects || []) {
      for (const rawObject of objectGroup.object_data || []) {
        this.ingestCacheObject(Buffer.from(rawObject, "base64"));
      }
    }
  }

  ingestCacheObject(buffer) {
    const party = this.proto.tryDecode("CSOCitadelParty", buffer);
    if (party?.party_id) {
      this.partyCache.set(String(party.party_id), party);
      if (party.party_id && party.match_id) {
        this.partyToMatch.set(String(party.party_id), String(party.match_id));
      }
      return;
    }

    const lobby = this.proto.tryDecode("CSOCitadelLobby", buffer);
    if (lobby?.lobby_id || lobby?.match_id) {
      const lobbyId = lobby.lobby_id ? String(lobby.lobby_id) : null;
      const matchId = lobby.match_id ? String(lobby.match_id) : null;
      if (lobbyId) {
        this.lobbyCache.set(lobbyId, lobby);
      }
      if (lobbyId && matchId) {
        this.partyToMatch.set(lobbyId, matchId);
      }
    }
  }

  normalizeWinningTeam(teamValue) {
    if (typeof teamValue === "number") {
      return teamValue === 1 ? 1 : 0;
    }
    if (typeof teamValue === "string" && teamValue.includes("Team1")) {
      return 1;
    }
    return 0;
  }

  responseCode(value) {
    if (typeof value === "number") {
      return value;
    }
    if (typeof value === "string") {
      if (value.endsWith("Success")) {
        return 1;
      }
      const direct = Number.parseInt(value, 10);
      if (!Number.isNaN(direct)) {
        return direct;
      }
    }
    return 0;
  }
}

class SteamBridgeApp {
  constructor() {
    this.proto = new ProtoBundle();
    this.store = new TaskStore(DB_PATH);
    this.adapter = null;
    this.lastTask = null;
  }

  async init() {
    await this.proto.load();
    const shouldUseSteam =
      process.env.STEAM_BRIDGE_MODE === "steam" ||
      !!process.env.STEAM_REFRESH_TOKEN ||
      fs.existsSync(REFRESH_TOKEN_PATH) ||
      (!!process.env.STEAM_USERNAME && !!process.env.STEAM_PASSWORD);

    this.adapter = shouldUseSteam ? new SteamGcAdapter(this.proto) : new MockGcAdapter();
    await this.adapter.connect();

    this.store.writeHeartbeat(this.getHeartbeatPayload());
    setInterval(() => {
      this.store.writeHeartbeat(this.getHeartbeatPayload());
    }, HEARTBEAT_INTERVAL_MS).unref();
  }

  getHeartbeatPayload() {
    return {
      runtime: {
        ...this.adapter.getRuntime(),
        poll_interval_ms: POLL_INTERVAL_MS,
        supported_task_types: SUPPORTED_TASK_TYPES,
        last_task: this.lastTask,
      },
    };
  }

  async run() {
    while (true) {
      try {
        const task = this.store.claimNextTask(SUPPORTED_TASK_TYPES);
        if (!task) {
          await sleep(POLL_INTERVAL_MS);
          continue;
        }

        this.lastTask = {
          id: task.id,
          type: task.type,
          started_at: nowMs(),
        };
        log("log", "Task uebernommen", { id: task.id, type: task.type });

        try {
          const result = await this.adapter.handleTask(task.type, task.payload);
          this.store.completeTask(task.id, result);
          log("log", "Task abgeschlossen", { id: task.id, type: task.type });
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          this.store.failTask(task.id, message);
          log("error", "Task fehlgeschlagen", { id: task.id, type: task.type, error: message });
        }

        this.lastTask.finished_at = nowMs();
        await sleep(TASK_DELAY_MS);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        log("error", "Worker-Loop Fehler", { error: message });
        await sleep(POLL_INTERVAL_MS);
      }
    }
  }
}

async function main() {
  const app = new SteamBridgeApp();
  await app.init();
  await app.run();
}

main().catch((error) => {
  const message = error instanceof Error ? error.stack || error.message : String(error);
  console.error(`[SteamBridge] Fatal: ${message}`);
  process.exitCode = 1;
});
