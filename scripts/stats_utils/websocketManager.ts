import WebSocket from 'ws';

export interface WebSocketManagerOptions {
  url: string;
  reconnectAttempts?: number;
  reconnectDelay?: number;
  maxReconnectDelay?: number;
  heartbeatInterval?: number;
  connectionTimeout?: number;
  onMessage: (data: any) => void;
  onConnect?: () => void;
  onDisconnect?: (code: number, reason: string) => void;
  onError?: (error: Error) => void;
  onReconnectAttempt?: (attempt: number) => void;
}

export class WebSocketManager {
  private ws: WebSocket | null = null;
  private url: string;
  private reconnectAttempts: number = 0;
  private maxReconnectAttempts: number;
  private reconnectDelay: number;
  private maxReconnectDelay: number;
  private heartbeatIntervalMs: number;
  private connectionTimeout: number;
  private heartbeatTimer: NodeJS.Timeout | null = null;
  private reconnectTimer: NodeJS.Timeout | null = null;
  private connectionTimer: NodeJS.Timeout | null = null;
  private lastPongTime: number = Date.now();
  private isConnecting: boolean = false;
  private shouldReconnect: boolean = true;
  private isClosed: boolean = false;

  // Callbacks
  private onMessage: (data: any) => void;
  private onConnect?: () => void;
  private onDisconnect?: (code: number, reason: string) => void;
  private onError?: (error: Error) => void;
  private onReconnectAttempt?: (attempt: number) => void;

  constructor(options: WebSocketManagerOptions) {
    this.url = options.url;
    this.maxReconnectAttempts = options.reconnectAttempts ?? Infinity;
    this.reconnectDelay = options.reconnectDelay ?? 1000;
    this.maxReconnectDelay = options.maxReconnectDelay ?? 30000;
    this.heartbeatIntervalMs = options.heartbeatInterval ?? 30000;
    this.connectionTimeout = options.connectionTimeout ?? 10000;
    this.onMessage = options.onMessage;
    this.onConnect = options.onConnect;
    this.onDisconnect = options.onDisconnect;
    this.onError = options.onError;
    this.onReconnectAttempt = options.onReconnectAttempt;
  }

  public connect(): void {
    if (this.isClosed) {
      throw new Error(
        'WebSocketManager has been closed and cannot be reconnected',
      );
    }
    this.shouldReconnect = true;
    this.connectInternal();
  }

  private connectInternal(): void {
    if (
      this.isConnecting ||
      (this.ws && this.ws.readyState === WebSocket.OPEN)
    ) {
      return; // Already connecting or connected
    }

    this.isConnecting = true;
    this.cleanup();

    console.log(
      `[WebSocket] Connecting to ${this.url} (attempt ${this.reconnectAttempts + 1})`,
    );

    try {
      this.ws = new WebSocket(this.url);

      // Set connection timeout
      this.connectionTimer = setTimeout(() => {
        console.error('[WebSocket] Connection timeout');
        if (this.ws && this.ws.readyState === WebSocket.CONNECTING) {
          this.ws.close();
        }
      }, this.connectionTimeout);

      this.ws.onopen = () => {
        console.log('[WebSocket] Connected successfully');
        this.clearConnectionTimeout();
        this.isConnecting = false;
        this.reconnectAttempts = 0;
        this.lastPongTime = Date.now();

        // Start heartbeat
        this.startHeartbeat();

        // Call user callback
        this.onConnect?.();
      };

      this.ws.onclose = (event) => {
        console.log(
          `[WebSocket] Closed: code=${event.code}, reason=${event.reason}`,
        );
        this.clearConnectionTimeout();
        this.isConnecting = false;

        // Call user callback
        this.onDisconnect?.(event.code, event.reason);

        // Schedule reconnection if needed
        if (this.shouldReconnect && !this.isClosed) {
          this.scheduleReconnect();
        }
      };

      this.ws.onerror = (event) => {
        console.error('[WebSocket] Error:', event.message);
        this.clearConnectionTimeout();
        this.isConnecting = false;

        // Call user callback
        this.onError?.(new Error(event.message));
      };

      this.ws.onmessage = (event) => {
        try {
          const data = JSON.parse(event.data.toString());
          this.onMessage(data);
        } catch (error) {
          console.error('[WebSocket] Failed to parse message:', error);
        }
      };

      this.ws.on('pong', () => {
        this.lastPongTime = Date.now();
        console.debug('[WebSocket] Received pong');
      });
    } catch (error) {
      console.error('[WebSocket] Connection error:', error);
      this.isConnecting = false;

      if (this.shouldReconnect && !this.isClosed) {
        this.scheduleReconnect();
      }
    }
  }

  private startHeartbeat(): void {
    this.stopHeartbeat();

    this.heartbeatTimer = setInterval(() => {
      if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
        console.log(
          '[WebSocket] Heartbeat: connection not open, stopping heartbeat',
        );
        this.stopHeartbeat();
        return;
      }

      const timeSinceLastPong = Date.now() - this.lastPongTime;
      if (timeSinceLastPong > this.heartbeatIntervalMs * 2) {
        console.error('[WebSocket] Heartbeat timeout - no pong received');
        this.ws.close();
        return;
      }

      try {
        this.ws.ping();
        console.debug('[WebSocket] Sent ping');
      } catch (error) {
        console.error('[WebSocket] Failed to send ping:', error);
      }
    }, this.heartbeatIntervalMs);
  }

  private stopHeartbeat(): void {
    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
  }

  private scheduleReconnect(): void {
    if (this.reconnectAttempts >= this.maxReconnectAttempts) {
      console.error('[WebSocket] Max reconnection attempts reached');
      return;
    }

    // Calculate exponential backoff delay
    const delay = Math.min(
      this.reconnectDelay * Math.pow(2, this.reconnectAttempts),
      this.maxReconnectDelay,
    );

    this.reconnectAttempts++;
    console.log(
      `[WebSocket] Scheduling reconnection in ${delay}ms (attempt ${this.reconnectAttempts})`,
    );

    // Call user callback
    this.onReconnectAttempt?.(this.reconnectAttempts);

    this.reconnectTimer = setTimeout(() => {
      this.connectInternal();
    }, delay);
  }

  private clearConnectionTimeout(): void {
    if (this.connectionTimer) {
      clearTimeout(this.connectionTimer);
      this.connectionTimer = null;
    }
  }

  private cleanup(): void {
    // Clear timers
    this.stopHeartbeat();
    this.clearConnectionTimeout();

    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    // Clean up WebSocket
    if (this.ws) {
      try {
        // Remove all event listeners
        this.ws.onopen = () => {};
        this.ws.onclose = () => {};
        this.ws.onerror = () => {};
        this.ws.onmessage = () => {};
        this.ws.removeAllListeners();

        // Close if still open
        if (
          this.ws.readyState === WebSocket.OPEN ||
          this.ws.readyState === WebSocket.CONNECTING
        ) {
          this.ws.close(1000, 'Closing for reconnection');
        }
      } catch (error) {
        console.error('[WebSocket] Error during cleanup:', error);
      }
      this.ws = null;
    }
  }

  public close(): void {
    console.log('[WebSocket] Closing connection permanently');
    this.isClosed = true;
    this.shouldReconnect = false;
    this.cleanup();
  }

  public isConnected(): boolean {
    return this.ws !== null && this.ws.readyState === WebSocket.OPEN;
  }

  public send(data: any): void {
    if (!this.isConnected()) {
      throw new Error('WebSocket is not connected');
    }

    try {
      const message = typeof data === 'string' ? data : JSON.stringify(data);
      this.ws!.send(message);
    } catch (error) {
      console.error('[WebSocket] Failed to send message:', error);
      throw error;
    }
  }
}
