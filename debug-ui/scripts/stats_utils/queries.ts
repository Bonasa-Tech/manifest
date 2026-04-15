// Database query constants

// ========== INSERT QUERIES ==========

export const INSERT_FILL_COMPLETE = `
  INSERT INTO fills_complete (
    slot, market, signature, taker, maker,
    taker_sequence_number, maker_sequence_number, fill_data
  )
  VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
  ON CONFLICT (signature, taker_sequence_number, maker_sequence_number)
  DO NOTHING
`;

export const INSERT_STATE_CHECKPOINT =
  'INSERT INTO state_checkpoints (last_fill_slot) VALUES ($1) RETURNING id';

export const INSERT_MARKET_CHECKPOINT =
  'INSERT INTO market_checkpoints (checkpoint_id, market, base_volume_checkpoints, quote_volume_checkpoints, checkpoint_timestamps, last_price) VALUES ($1, $2, $3, $4, $5, $6)';

export const INSERT_TRADER_STATS =
  'INSERT INTO trader_stats (checkpoint_id, trader, num_taker_trades, num_maker_trades, taker_notional_volume, maker_notional_volume) VALUES ($1, $2, $3, $4, $5, $6)';

export const INSERT_TRADER_POSITION =
  'INSERT INTO trader_positions (checkpoint_id, trader, mint, position, acquisition_value) VALUES ($1, $2, $3, $4, $5)';

// ========== SELECT QUERIES ==========

export const SELECT_ALT_MARKETS = 'SELECT alt, market FROM alt_markets';

// Select the most recent checkpoint that actually has market data
// This handles cases where a checkpoint was created but market data save failed/was interrupted
export const SELECT_RECENT_CHECKPOINT = `
  SELECT sc.id, sc.last_fill_slot
  FROM state_checkpoints sc
  WHERE EXISTS (SELECT 1 FROM market_checkpoints mc WHERE mc.checkpoint_id = sc.id)
  ORDER BY sc.created_at DESC
  LIMIT 1
`;

export const SELECT_MARKET_CHECKPOINTS =
  'SELECT market, base_volume_checkpoints, quote_volume_checkpoints, checkpoint_timestamps, last_price FROM market_checkpoints WHERE checkpoint_id = $1';

// Select the freshest checkpoint data for each market across ALL checkpoints
// Uses DISTINCT ON to get only the row with the highest checkpoint_id per market
export const SELECT_FRESHEST_MARKET_CHECKPOINTS = `
  SELECT DISTINCT ON (market)
    market,
    base_volume_checkpoints,
    quote_volume_checkpoints,
    checkpoint_timestamps,
    last_price,
    checkpoint_id
  FROM market_checkpoints
  ORDER BY market, checkpoint_id DESC
`;

export const SELECT_FILLS_COMPLETE_COUNT_BASE =
  'SELECT COUNT(*) as total FROM fills_complete';

export const SELECT_FILLS_COMPLETE_DATA_BASE =
  'SELECT fill_data FROM fills_complete';

// Select the first (oldest) fill for a market to determine market creation time
// Uses existing (market, timestamp DESC) index - B-tree allows efficient seek to range end
export const SELECT_FIRST_FILL_FOR_MARKET = `
  SELECT fill_data FROM fills_complete
  WHERE market = $1
  ORDER BY timestamp ASC
  LIMIT 1
`;

// ========== DELETE QUERIES ==========

export const DELETE_OLD_CHECKPOINTS =
  'DELETE FROM state_checkpoints WHERE id != $1';

export const DELETE_OLD_MARKET_VOLUMES =
  'DELETE FROM market_volumes WHERE checkpoint_id != $1';

// ========== TRANSACTION QUERIES ==========

export const BEGIN_TRANSACTION = 'BEGIN';
export const COMMIT_TRANSACTION = 'COMMIT';
export const ROLLBACK_TRANSACTION = 'ROLLBACK';

// ========== CREATE TABLE QUERIES ==========

export const CREATE_STATE_CHECKPOINTS_TABLE = `
  CREATE TABLE IF NOT EXISTS state_checkpoints (
    id SERIAL PRIMARY KEY,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    last_fill_slot BIGINT NOT NULL
  )
`;

// Deprecated: volume since last checkpoint is now handled by market_checkpoints table
export const CREATE_MARKET_VOLUMES_TABLE = `
  CREATE TABLE IF NOT EXISTS market_volumes (
    checkpoint_id INTEGER REFERENCES state_checkpoints(id) ON DELETE CASCADE,
    market TEXT NOT NULL,
    base_volume_since_last_checkpoint NUMERIC,
    quote_volume_since_last_checkpoint NUMERIC,
    PRIMARY KEY (checkpoint_id, market)
  )
`;

export const CREATE_MARKET_CHECKPOINTS_TABLE = `
  CREATE TABLE IF NOT EXISTS market_checkpoints (
    checkpoint_id INTEGER REFERENCES state_checkpoints(id) ON DELETE CASCADE,
    market TEXT NOT NULL,
    base_volume_checkpoints JSONB NOT NULL,
    quote_volume_checkpoints JSONB NOT NULL,
    checkpoint_timestamps JSONB NOT NULL,
    last_price NUMERIC,
    PRIMARY KEY (checkpoint_id, market)
  )
`;

export const CREATE_TRADER_STATS_TABLE = `
  CREATE TABLE IF NOT EXISTS trader_stats (
    checkpoint_id INTEGER REFERENCES state_checkpoints(id) ON DELETE CASCADE,
    trader TEXT NOT NULL,
    num_taker_trades INTEGER DEFAULT 0,
    num_maker_trades INTEGER DEFAULT 0,
    taker_notional_volume NUMERIC DEFAULT 0,
    maker_notional_volume NUMERIC DEFAULT 0,
    PRIMARY KEY (checkpoint_id, trader)
  )
`;

// TODO: Remove. No longer used.
export const CREATE_FILL_LOG_RESULTS_TABLE = `
  CREATE TABLE IF NOT EXISTS fill_log_results (
    checkpoint_id INTEGER REFERENCES state_checkpoints(id) ON DELETE CASCADE,
    market TEXT NOT NULL,
    fill_data JSONB NOT NULL,
    PRIMARY KEY (checkpoint_id, market)
  )
`;

export const CREATE_TRADER_POSITIONS_TABLE = `
  CREATE TABLE IF NOT EXISTS trader_positions (
    checkpoint_id INTEGER REFERENCES state_checkpoints(id) ON DELETE CASCADE,
    trader TEXT NOT NULL,
    mint TEXT NOT NULL,
    position NUMERIC NOT NULL,
    acquisition_value NUMERIC NOT NULL,
    PRIMARY KEY (checkpoint_id, trader, mint)
  )
`;

export const CREATE_FILLS_COMPLETE_TABLE = `
  CREATE TABLE IF NOT EXISTS fills_complete (
    id BIGSERIAL PRIMARY KEY,
    slot BIGINT NOT NULL,
    market TEXT NOT NULL,
    signature TEXT NOT NULL,
    taker TEXT NOT NULL,
    maker TEXT NOT NULL,
    taker_sequence_number BIGINT NOT NULL,
    maker_sequence_number BIGINT NOT NULL,
    fill_data JSONB NOT NULL,
    timestamp TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,

    -- Optimal deduplication using signature + sequence numbers
    CONSTRAINT unique_complete_fill UNIQUE (signature, taker_sequence_number, maker_sequence_number)
  )
`;

export const CREATE_FILLS_COMPLETE_INDEXES = `
  CREATE INDEX IF NOT EXISTS idx_fills_complete_market_timestamp ON fills_complete (market, timestamp DESC);
  CREATE INDEX IF NOT EXISTS idx_fills_complete_slot ON fills_complete (slot);
  CREATE INDEX IF NOT EXISTS idx_fills_complete_signature ON fills_complete (signature);
  CREATE INDEX IF NOT EXISTS idx_fills_complete_taker ON fills_complete (taker);
  CREATE INDEX IF NOT EXISTS idx_fills_complete_maker ON fills_complete (maker);
`;

export const CREATE_ALT_MARKETS_TABLE = `
  CREATE TABLE IF NOT EXISTS alt_markets (
    alt TEXT NOT NULL,
    market TEXT NOT NULL,
    PRIMARY KEY (alt, market)
  )
`;

// ========== ALTER TABLE QUERIES ==========

export const ALTER_MARKET_CHECKPOINTS_ADD_TIMESTAMPS = `
  ALTER TABLE market_checkpoints 
  ADD COLUMN IF NOT EXISTS checkpoint_timestamps JSONB;
`;
