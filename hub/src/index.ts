// Flint Hub: share Flint games and animations.
// Every /api request goes to one SQLite-backed Durable Object that holds accounts, movies, likes and comments.

import { DurableObject } from "cloudflare:workers";

interface Env {
  HUB: DurableObjectNamespace<Hub>;
}

const MAX_FILE = 1_900_000; // DO SQLite values are capped at 2 MB
const MAX_THUMB = 400_000;
const UPLOADS_PER_DAY = 40;
const PAGE = 24;
const KINDS = ["game", "animation"];

const cors = { "Access-Control-Allow-Origin": "*", "Access-Control-Allow-Headers": "Authorization, Content-Type", "Access-Control-Allow-Methods": "GET, POST, DELETE, OPTIONS" };

const json = (data: unknown, status = 200) => Response.json(data, { status, headers: { ...cors, "Cache-Control": "no-store" } });
const err = (msg: string, status = 400) => json({ error: msg }, status);

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    if (req.method === "OPTIONS") return new Response(null, { headers: cors });
    const url = new URL(req.url);
    if (url.pathname === "/" || url.pathname === "") {
      return new Response("Flint Hub — share Flint games and animations.\nBrowse it from Flint Player (right-click → Browse Flint Hub).\nAPI: /api/movies\n", {
        headers: { "Content-Type": "text/plain; charset=utf-8", ...cors },
      });
    }
    if (!url.pathname.startsWith("/api/")) return err("not found", 404);
    return env.HUB.get(env.HUB.idFromName("hub")).fetch(req);
  },
};

type Row = Record<string, SqlStorageValue | boolean>;

function b64decode(s: string): Uint8Array {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

function hex(buf: ArrayBuffer | Uint8Array): string {
  return [...new Uint8Array(buf)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

async function hashPassword(password: string, salt: string): Promise<string> {
  const key = await crypto.subtle.importKey("raw", new TextEncoder().encode(password), "PBKDF2", false, ["deriveBits"]);
  const bits = await crypto.subtle.deriveBits({ name: "PBKDF2", hash: "SHA-256", salt: new TextEncoder().encode(salt), iterations: 100_000 }, key, 256);
  return hex(bits);
}

function clean(s: unknown, max: number): string {
  return typeof s === "string" ? s.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, "").trim().slice(0, max) : "";
}

const MOVIE_COLS = "id, author, title, description, kind, tags, created, updated, plays, likes, size, width, height, fps, frames, (SELECT COUNT(*) FROM comments c WHERE c.movie = movies.id) AS comments";

export class Hub extends DurableObject<Env> {
  sql: SqlStorage;

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.sql.exec(`
      CREATE TABLE IF NOT EXISTS users (name TEXT PRIMARY KEY COLLATE NOCASE, hash TEXT NOT NULL, salt TEXT NOT NULL, created INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS sessions (token TEXT PRIMARY KEY, name TEXT NOT NULL, created INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS movies (
        id INTEGER PRIMARY KEY AUTOINCREMENT, author TEXT NOT NULL, title TEXT NOT NULL, description TEXT NOT NULL,
        kind TEXT NOT NULL, tags TEXT NOT NULL, created INTEGER NOT NULL, updated INTEGER NOT NULL,
        plays INTEGER NOT NULL DEFAULT 0, likes INTEGER NOT NULL DEFAULT 0, size INTEGER NOT NULL,
        width REAL NOT NULL, height REAL NOT NULL, fps REAL NOT NULL, frames INTEGER NOT NULL,
        file BLOB NOT NULL, thumb BLOB
      );
      CREATE INDEX IF NOT EXISTS movies_created ON movies(created);
      CREATE TABLE IF NOT EXISTS likes (movie INTEGER NOT NULL, name TEXT NOT NULL COLLATE NOCASE, PRIMARY KEY (movie, name));
      CREATE TABLE IF NOT EXISTS comments (id INTEGER PRIMARY KEY AUTOINCREMENT, movie INTEGER NOT NULL, author TEXT NOT NULL, body TEXT NOT NULL, created INTEGER NOT NULL);
      CREATE INDEX IF NOT EXISTS comments_movie ON comments(movie);
    `);
  }

  user(req: Request): string | null {
    const auth = req.headers.get("Authorization") ?? "";
    const token = auth.startsWith("Bearer ") ? auth.slice(7) : "";
    if (!token) return null;
    const row = this.sql.exec("SELECT name FROM sessions WHERE token = ?", token).toArray()[0];
    return row ? String(row.name) : null;
  }

  newSession(name: string) {
    const token = hex(crypto.getRandomValues(new Uint8Array(24)));
    this.sql.exec("INSERT INTO sessions (token, name, created) VALUES (?, ?, ?)", token, name, Date.now());
    return json({ token, name });
  }

  movie(id: number, me: string | null): Row | null {
    const found = this.sql.exec(`SELECT ${MOVIE_COLS} FROM movies WHERE id = ?`, id).toArray()[0];
    if (!found) return null;
    const row: Row = { ...found };
    row.liked = me ? this.sql.exec("SELECT 1 FROM likes WHERE movie = ? AND name = ?", id, me).toArray().length > 0 : false;
    return row;
  }

  async fetch(req: Request): Promise<Response> {
    try {
      return await this.route(req);
    } catch (e) {
      return err(`server error: ${(e as Error).message}`, 500);
    }
  }

  async route(req: Request): Promise<Response> {
    const url = new URL(req.url);
    const parts = url.pathname.split("/").filter(Boolean).slice(1); // drop "api"
    const me = this.user(req);
    const body = async (): Promise<Record<string, unknown>> => {
      try {
        return (await req.json()) as Record<string, unknown>;
      } catch {
        return {};
      }
    };

    // ---- accounts
    if (req.method === "POST" && parts[0] === "register") {
      const b = await body();
      const name = clean(b.name, 24);
      const password = typeof b.password === "string" ? b.password : "";
      if (!/^[A-Za-z0-9_\-]{3,24}$/.test(name)) return err("username must be 3-24 letters, numbers, _ or -");
      if (password.length < 6) return err("password must be at least 6 characters");
      if (this.sql.exec("SELECT 1 FROM users WHERE name = ?", name).toArray().length) return err("that username is taken", 409);
      const salt = hex(crypto.getRandomValues(new Uint8Array(16)));
      this.sql.exec("INSERT INTO users (name, hash, salt, created) VALUES (?, ?, ?, ?)", name, await hashPassword(password, salt), salt, Date.now());
      return this.newSession(name);
    }
    if (req.method === "POST" && parts[0] === "login") {
      const b = await body();
      const name = clean(b.name, 24);
      const row = this.sql.exec("SELECT name, hash, salt FROM users WHERE name = ?", name).toArray()[0];
      if (!row || (await hashPassword(String(b.password ?? ""), String(row.salt))) !== row.hash) return err("wrong username or password", 401);
      return this.newSession(String(row.name));
    }
    if (req.method === "GET" && parts[0] === "me") {
      return me ? json({ name: me }) : err("not logged in", 401);
    }

    if (parts[0] !== "movies") return err("not found", 404);
    const id = parts[1] ? Number(parts[1]) : NaN;

    // ---- list / search
    if (req.method === "GET" && parts.length === 1) {
      const where: string[] = [];
      const args: SqlStorageValue[] = [];
      const kind = url.searchParams.get("kind");
      if (kind && KINDS.includes(kind)) {
        where.push("kind = ?");
        args.push(kind);
      }
      const author = url.searchParams.get("author");
      if (author) {
        where.push("author = ? COLLATE NOCASE");
        args.push(author);
      }
      const q = clean(url.searchParams.get("q"), 80);
      if (q) {
        where.push("(title LIKE ? OR description LIKE ? OR tags LIKE ? OR author LIKE ?)");
        const like = `%${q.replace(/[%_]/g, "")}%`;
        args.push(like, like, like, like);
      }
      const order = { top: "likes DESC, plays DESC", plays: "plays DESC", new: "created DESC" }[url.searchParams.get("sort") ?? "new"] ?? "created DESC";
      const page = Math.max(0, Number(url.searchParams.get("page") ?? 0) | 0);
      const rows = this.sql
        .exec(`SELECT ${MOVIE_COLS} FROM movies ${where.length ? "WHERE " + where.join(" AND ") : ""} ORDER BY ${order}, id DESC LIMIT ? OFFSET ?`, ...args, PAGE + 1, page * PAGE)
        .toArray();
      return json({ movies: rows.slice(0, PAGE), more: rows.length > PAGE });
    }

    // ---- publish (new or update own)
    if (req.method === "POST" && parts.length === 1) {
      if (!me) return err("log in to publish", 401);
      const b = await body();
      const title = clean(b.title, 80);
      if (!title) return err("give it a title");
      const kind = KINDS.includes(String(b.kind)) ? String(b.kind) : "animation";
      let file: Uint8Array, thumb: Uint8Array | null;
      try {
        file = b64decode(String(b.file ?? ""));
        thumb = b.thumb ? b64decode(String(b.thumb)) : null;
      } catch {
        return err("bad upload encoding");
      }
      if (file.length < 8 || file[0] !== 0x46 || file[1] !== 0x4c || file[2] !== 0x58) return err("file is not a Flint movie (.flx)");
      if (file.length > MAX_FILE) return err(`movie is too big (max ${(MAX_FILE / 1e6).toFixed(1)} MB)`, 413);
      if (thumb && (thumb.length > MAX_THUMB || thumb[1] !== 0x50 || thumb[2] !== 0x4e || thumb[3] !== 0x47)) thumb = null;
      const meta = [title, clean(b.description, 2000), kind, clean(b.tags, 200), Date.now(), file.length, Number(b.width) || 0, Number(b.height) || 0, Number(b.fps) || 24, Number(b.frames) | 0];
      const update = Number(b.id);
      if (update) {
        const own = this.sql.exec("SELECT author FROM movies WHERE id = ?", update).toArray()[0];
        if (!own) return err("movie not found", 404);
        if (String(own.author).toLowerCase() !== me.toLowerCase()) return err("you can only update your own movies", 403);
        this.sql.exec(
          "UPDATE movies SET title = ?, description = ?, kind = ?, tags = ?, updated = ?, size = ?, width = ?, height = ?, fps = ?, frames = ?, file = ?, thumb = ? WHERE id = ?",
          ...meta, file.buffer as ArrayBuffer, thumb ? (thumb.buffer as ArrayBuffer) : null, update,
        );
        return json({ id: update, updated: true });
      }
      const dayAgo = Date.now() - 86_400_000;
      const recent = Number(this.sql.exec("SELECT COUNT(*) AS n FROM movies WHERE author = ? AND created > ?", me, dayAgo).one().n);
      if (recent >= UPLOADS_PER_DAY) return err("upload limit reached for today", 429);
      const now = Date.now();
      const row = this.sql
        .exec(
          "INSERT INTO movies (title, description, kind, tags, updated, size, width, height, fps, frames, file, thumb, author, created) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
          ...meta, file.buffer as ArrayBuffer, thumb ? (thumb.buffer as ArrayBuffer) : null, me, now,
        )
        .one();
      return json({ id: row.id });
    }

    if (!Number.isFinite(id)) return err("not found", 404);
    const sub = parts[2];

    if (req.method === "GET" && !sub) {
      const m = this.movie(id, me);
      return m ? json(m) : err("movie not found", 404);
    }
    if (req.method === "GET" && (sub === "file" || sub === "thumb")) {
      const row = this.sql.exec(`SELECT ${sub} AS data FROM movies WHERE id = ?`, id).toArray()[0];
      if (!row || row.data === null) return err("not found", 404);
      if (sub === "file" && url.searchParams.get("play") !== "0") this.sql.exec("UPDATE movies SET plays = plays + 1 WHERE id = ?", id);
      return new Response(row.data as ArrayBuffer, {
        headers: { ...cors, "Content-Type": sub === "file" ? "application/x-flint-movie" : "image/png", "Cache-Control": sub === "thumb" ? "public, max-age=300" : "no-store" },
      });
    }
    if (req.method === "DELETE" && !sub) {
      if (!me) return err("log in first", 401);
      const own = this.sql.exec("SELECT author FROM movies WHERE id = ?", id).toArray()[0];
      if (!own) return err("movie not found", 404);
      if (String(own.author).toLowerCase() !== me.toLowerCase()) return err("you can only delete your own movies", 403);
      this.sql.exec("DELETE FROM movies WHERE id = ?", id);
      this.sql.exec("DELETE FROM likes WHERE movie = ?", id);
      this.sql.exec("DELETE FROM comments WHERE movie = ?", id);
      return json({ deleted: true });
    }
    if (req.method === "POST" && sub === "like") {
      if (!me) return err("log in to like", 401);
      if (!this.sql.exec("SELECT 1 FROM movies WHERE id = ?", id).toArray().length) return err("movie not found", 404);
      const liked = this.sql.exec("SELECT 1 FROM likes WHERE movie = ? AND name = ?", id, me).toArray().length > 0;
      if (liked) this.sql.exec("DELETE FROM likes WHERE movie = ? AND name = ?", id, me);
      else this.sql.exec("INSERT INTO likes (movie, name) VALUES (?, ?)", id, me);
      const likes = Number(this.sql.exec("SELECT COUNT(*) AS n FROM likes WHERE movie = ?", id).one().n);
      this.sql.exec("UPDATE movies SET likes = ? WHERE id = ?", likes, id);
      return json({ liked: !liked, likes });
    }
    if (sub === "comments") {
      if (req.method === "GET") {
        return json({ comments: this.sql.exec("SELECT id, author, body, created FROM comments WHERE movie = ? ORDER BY id DESC LIMIT 200", id).toArray() });
      }
      if (req.method === "POST") {
        if (!me) return err("log in to comment", 401);
        if (!this.sql.exec("SELECT 1 FROM movies WHERE id = ?", id).toArray().length) return err("movie not found", 404);
        const text = clean((await body()).body, 1000);
        if (!text) return err("comment is empty");
        const row = this.sql.exec("INSERT INTO comments (movie, author, body, created) VALUES (?, ?, ?, ?) RETURNING id", id, me, text, Date.now()).one();
        return json({ id: row.id });
      }
    }
    return err("not found", 404);
  }
}
