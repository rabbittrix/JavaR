import * as net from "net";

/** Wire protocol constants — must match javar-core / javar-agent. */
const MAGIC = 0x4a415652;
const VERSION = 1;
const KIND_PING = 1;
const KIND_PONG = 2;
const KIND_STATUS = 3;
const KIND_HOT_DEPLOY = 8;
const KIND_TELEMETRY = 7;

export interface TelemetrySnapshot {
  java_heap_used: number;
  java_heap_max: number;
  javar_managed: number;
  reload_count: number;
  loaded_classes?: number;
}

export class JavaRClient {
  private socket: net.Socket | undefined;
  private buffer = Buffer.alloc(0);

  constructor(
    private readonly host: string,
    private readonly port: number
  ) {}

  isConnected(): boolean {
    return !!this.socket && !this.socket.destroyed;
  }

  connect(): Promise<void> {
    return new Promise((resolve, reject) => {
      const socket = net.createConnection({ host: this.host, port: this.port }, () => {
        this.socket = socket;
        resolve();
      });
      socket.on("error", reject);
      socket.on("data", (chunk) => {
        this.buffer = Buffer.concat([this.buffer, chunk]);
      });
      socket.on("close", () => {
        this.socket = undefined;
      });
    });
  }

  close(): void {
    this.socket?.destroy();
    this.socket = undefined;
    this.buffer = Buffer.alloc(0);
  }

  async hotDeploy(filePath: string): Promise<void> {
    const payload = Buffer.from(
      JSON.stringify({ state: "hot_deploy", detail: filePath }),
      "utf8"
    );
    await this.request(KIND_HOT_DEPLOY, payload);
  }

  async telemetry(): Promise<TelemetrySnapshot> {
    const frame = await this.request(KIND_TELEMETRY, Buffer.alloc(0));
    return JSON.parse(frame.toString("utf8")) as TelemetrySnapshot;
  }

  async ping(): Promise<boolean> {
    const kind = await this.requestKind(KIND_PING, Buffer.alloc(0));
    return kind === KIND_PONG || kind === KIND_STATUS;
  }

  private request(kind: number, payload: Buffer): Promise<Buffer> {
    return new Promise((resolve, reject) => {
      if (!this.socket) {
        reject(new Error("not connected"));
        return;
      }
      const frame = encodeFrame(kind, payload);
      const onData = () => {
        const decoded = tryDecode(this.buffer);
        if (!decoded) {
          return;
        }
        this.buffer = decoded.rest;
        this.socket?.off("data", onData);
        if (decoded.kind === 4 /* ERROR */) {
          reject(new Error(decoded.payload.toString("utf8")));
        } else {
          resolve(decoded.payload);
        }
      };
      this.socket.on("data", onData);
      this.socket.write(frame, (err) => {
        if (err) {
          this.socket?.off("data", onData);
          reject(err);
        }
      });
      setTimeout(() => {
        this.socket?.off("data", onData);
        reject(new Error("JavaR request timeout"));
      }, 5000);
    });
  }

  private async requestKind(kind: number, payload: Buffer): Promise<number> {
    return new Promise((resolve, reject) => {
      if (!this.socket) {
        reject(new Error("not connected"));
        return;
      }
      const frame = encodeFrame(kind, payload);
      const onData = () => {
        const decoded = tryDecode(this.buffer);
        if (!decoded) {
          return;
        }
        this.buffer = decoded.rest;
        this.socket?.off("data", onData);
        resolve(decoded.kind);
      };
      this.socket.on("data", onData);
      this.socket.write(frame, (err) => {
        if (err) {
          this.socket?.off("data", onData);
          reject(err);
        }
      });
    });
  }
}

function encodeFrame(kind: number, payload: Buffer): Buffer {
  const header = Buffer.alloc(10);
  header.writeUInt32LE(MAGIC, 0);
  header.writeUInt8(VERSION, 4);
  header.writeUInt8(kind, 5);
  header.writeUInt32LE(payload.length, 6);
  return Buffer.concat([header, payload]);
}

function tryDecode(
  buf: Buffer
): { kind: number; payload: Buffer; rest: Buffer } | undefined {
  if (buf.length < 10) {
    return undefined;
  }
  const magic = buf.readUInt32LE(0);
  if (magic !== MAGIC) {
    return undefined;
  }
  const kind = buf.readUInt8(5);
  const len = buf.readUInt32LE(6);
  if (buf.length < 10 + len) {
    return undefined;
  }
  return {
    kind,
    payload: buf.subarray(10, 10 + len),
    rest: buf.subarray(10 + len),
  };
}
