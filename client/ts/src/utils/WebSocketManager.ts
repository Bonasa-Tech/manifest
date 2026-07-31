import WebSocket from 'ws';

/**
 * Manages WebSocket server with client heartbeat functionality
 */
export class WebSocketManager {
  private static readonly MAX_CLIENTS = 100;
  private static readonly MAX_CLIENTS_PER_IP = 10;
  private static readonly MAX_PAYLOAD_BYTES = 64 * 1024;
  private static readonly MAX_BUFFERED_BYTES = 1024 * 1024;
  private wss: WebSocket.Server;
  private clientHeartbeats: Map<WebSocket, NodeJS.Timeout> = new Map();
  private clientsByIp: Map<string, number> = new Map();
  private heartbeatInterval: number;

  constructor(
    port: number,
    heartbeatInterval: number = 30000,
    host: string = process.env.FILL_FEED_WS_HOST ?? '127.0.0.1',
  ) {
    this.heartbeatInterval = heartbeatInterval;
    this.wss = new WebSocket.Server({
      // Bind locally by default and cap frames to keep this unauthenticated
      // broadcast endpoint from becoming a public memory/connection sink.
      port,
      host,
      maxPayload: WebSocketManager.MAX_PAYLOAD_BYTES,
    });

    this.wss.on('connection', (ws: WebSocket, request) => {
      const ip = request.socket.remoteAddress ?? 'unknown';
      const ipClients = this.clientsByIp.get(ip) ?? 0;
      if (
        this.wss.clients.size > WebSocketManager.MAX_CLIENTS ||
        ipClients >= WebSocketManager.MAX_CLIENTS_PER_IP
      ) {
        ws.close(1013, 'Server capacity reached');
        return;
      }
      this.clientsByIp.set(ip, ipClients + 1);
      console.log('New client connected');

      // Start heartbeat for this client
      this.startClientHeartbeat(ws);

      ws.on('message', () => {
        ws.close(1008, 'Inbound messages are not supported');
      });

      ws.on('pong', () => {
        // Client is still alive, reset the heartbeat timer
        this.resetClientHeartbeat(ws);
      });

      ws.on('close', () => {
        console.log('Client disconnected');
        this.stopClientHeartbeat(ws);
        const remaining = (this.clientsByIp.get(ip) ?? 1) - 1;
        if (remaining > 0) {
          this.clientsByIp.set(ip, remaining);
        } else {
          this.clientsByIp.delete(ip);
        }
      });

      ws.on('error', (error) => {
        console.error('WebSocket error:', error);
        this.stopClientHeartbeat(ws);
      });
    });
  }

  /**
   * Broadcast a message to all connected clients
   */
  public broadcast(message: string): void {
    this.wss.clients.forEach((client) => {
      if (client.readyState === WebSocket.OPEN) {
        // Disconnect slow consumers before queued broadcasts grow without bound.
        if (client.bufferedAmount > WebSocketManager.MAX_BUFFERED_BYTES) {
          client.terminate();
          this.stopClientHeartbeat(client);
          return;
        }
        client.send(message);
      }
    });
  }

  /**
   * Close the WebSocket server and clean up
   */
  public close(): void {
    // Clean up all heartbeats before closing
    this.clientHeartbeats.forEach((timeout, ws) => {
      clearTimeout(timeout);
      ws.close();
    });
    this.clientHeartbeats.clear();
    this.wss.close();
  }

  private startClientHeartbeat(ws: WebSocket): void {
    const timeout = setTimeout(() => {
      console.log('Client heartbeat timeout, closing connection');
      ws.terminate();
      this.clientHeartbeats.delete(ws);
    }, this.heartbeatInterval * 2); // Wait for 2x heartbeat interval before considering dead

    this.clientHeartbeats.set(ws, timeout);

    // Send initial ping
    if (ws.readyState === WebSocket.OPEN) {
      ws.ping();
    }
  }

  private resetClientHeartbeat(ws: WebSocket): void {
    // Clear existing timeout
    const existingTimeout = this.clientHeartbeats.get(ws);
    if (existingTimeout) {
      clearTimeout(existingTimeout);
    }

    // Set new timeout and send next ping
    const timeout = setTimeout(() => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.ping();
        // Set another timeout for the pong response
        const pongTimeout = setTimeout(() => {
          console.log('Client pong timeout, closing connection');
          ws.terminate();
          this.clientHeartbeats.delete(ws);
        }, this.heartbeatInterval);
        this.clientHeartbeats.set(ws, pongTimeout);
      }
    }, this.heartbeatInterval);

    this.clientHeartbeats.set(ws, timeout);
  }

  private stopClientHeartbeat(ws: WebSocket): void {
    const timeout = this.clientHeartbeats.get(ws);
    if (timeout) {
      clearTimeout(timeout);
      this.clientHeartbeats.delete(ws);
    }
  }
}
