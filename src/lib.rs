//! Market maker bot simulator.
//!
//! Features:
//! - Spread quoting around mid-price
//! - Inventory risk management (skew quotes based on position)
//! - PnL tracking (realized + unrealized)
//! - Fill simulation
//! - Volatility-adjusted spreads
//! - Max position limits

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Side { Buy, Sell }

#[derive(Debug, Clone)]
pub struct Quote {
    pub bid_price: f64,
    pub ask_price: f64,
    pub bid_size: f64,
    pub ask_size: f64,
    pub timestamp: u64,
}

#[derive(Debug, Clone)]
pub struct Fill {
    pub side: Side,
    pub price: f64,
    pub quantity: f64,
    pub timestamp: u64,
}

#[derive(Debug, Clone)]
pub struct MarketMakerConfig {
    pub base_spread_bps: f64,      // base spread in basis points
    pub inventory_skew_factor: f64, // how much to skew based on inventory
    pub max_position: f64,          // max absolute position
    pub order_size: f64,            // size per quote
    pub volatility_factor: f64,     // spread multiplier for volatility
}

impl Default for MarketMakerConfig {
    fn default() -> Self {
        Self {
            base_spread_bps: 10.0,
            inventory_skew_factor: 0.5,
            max_position: 100.0,
            order_size: 1.0,
            volatility_factor: 1.0,
        }
    }
}

#[derive(Debug)]
pub enum MmError {
    MaxPositionReached,
    InvalidPrice,
    InvalidSize,
}

impl fmt::Display for MmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MmError::MaxPositionReached => write!(f, "max position reached"),
            MmError::InvalidPrice => write!(f, "invalid price"),
            MmError::InvalidSize => write!(f, "invalid size"),
        }
    }
}

pub struct MarketMaker {
    config: MarketMakerConfig,
    position: f64,        // positive = long, negative = short
    cash: f64,             // cash balance
    avg_entry: f64,        // average entry price
    realized_pnl: f64,
    fills: Vec<Fill>,
    mid_price: f64,
    volatility: f64,       // realized volatility estimate
    quote_count: u64,
}

impl MarketMaker {
    pub fn new(config: MarketMakerConfig, initial_cash: f64) -> Self {
        Self {
            config,
            position: 0.0,
            cash: initial_cash,
            avg_entry: 0.0,
            realized_pnl: 0.0,
            fills: Vec::new(),
            mid_price: 0.0,
            volatility: 0.0,
            quote_count: 0,
        }
    }

    pub fn update_mid_price(&mut self, mid: f64, volatility: f64) {
        self.mid_price = mid;
        self.volatility = volatility;
    }

    /// Generate a two-sided quote.
    pub fn quote(&mut self) -> Result<Quote, MmError> {
        if self.mid_price <= 0.0 { return Err(MmError::InvalidPrice); }
        if self.config.order_size <= 0.0 { return Err(MmError::InvalidSize); }

        // Volatility-adjusted spread
        let vol_spread = self.config.base_spread_bps * (1.0 + self.volatility * self.config.volatility_factor);
        let half_spread = self.mid_price * vol_spread / 10000.0 / 2.0;

        // Inventory skew
        let skew = self.position * self.config.inventory_skew_factor * (self.mid_price / self.config.max_position);
        let bid_price = self.mid_price - half_spread - skew;
        let ask_price = self.mid_price + half_spread - skew;

        // Size adjustment based on position limits
        let bid_size = if self.position >= self.config.max_position {
            0.0
        } else {
            self.config.order_size
        };
        let ask_size = if self.position <= -self.config.max_position {
            0.0
        } else {
            self.config.order_size
        };

        self.quote_count += 1;
        Ok(Quote {
            bid_price,
            ask_price,
            bid_size,
            ask_size,
            timestamp: self.quote_count,
        })
    }

    /// Simulate a fill on one side.
    pub fn on_fill(&mut self, side: Side, price: f64, quantity: f64, timestamp: u64) -> Result<(), MmError> {
        if quantity <= 0.0 { return Err(MmError::InvalidSize); }
        if price <= 0.0 { return Err(MmError::InvalidPrice); }

        match side {
            Side::Buy => {
                if self.position + quantity > self.config.max_position {
                    return Err(MmError::MaxPositionReached);
                }
                // Update average entry
                if self.position >= 0.0 {
                    let total = self.position + quantity;
                    self.avg_entry = (self.avg_entry * self.position + price * quantity) / total;
                } else {
                    // Closing short
                    let close_qty = quantity.min(-self.position);
                    self.realized_pnl += (self.avg_entry - price) * close_qty;
                    if quantity > close_qty {
                        self.avg_entry = price;
                    }
                }
                self.position += quantity;
                self.cash -= price * quantity;
            }
            Side::Sell => {
                if self.position - quantity < -self.config.max_position {
                    return Err(MmError::MaxPositionReached);
                }
                if self.position > 0.0 {
                    let close_qty = quantity.min(self.position);
                    self.realized_pnl += (price - self.avg_entry) * close_qty;
                    if quantity > close_qty {
                        self.avg_entry = price;
                    }
                } else {
                    let total = -self.position + quantity;
                    self.avg_entry = (self.avg_entry * (-self.position) + price * quantity) / total;
                }
                self.position -= quantity;
                self.cash += price * quantity;
            }
        }

        self.fills.push(Fill { side, price, quantity, timestamp });
        Ok(())
    }

    pub fn position(&self) -> f64 { self.position }
    pub fn cash(&self) -> f64 { self.cash }
    pub fn realized_pnl(&self) -> f64 { self.realized_pnl }
    pub fn unrealized_pnl(&self) -> f64 {
        if self.position == 0.0 { return 0.0; }
        (self.mid_price - self.avg_entry) * self.position
    }
    pub fn total_pnl(&self) -> f64 { self.realized_pnl + self.unrealized_pnl() }
    pub fn fills(&self) -> &[Fill] { &self.fills }
    pub fn quote_count(&self) -> u64 { self.quote_count }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_mm() {
        let mm = MarketMaker::new(MarketMakerConfig::default(), 10000.0);
        assert_eq!(mm.position(), 0.0);
        assert_eq!(mm.cash(), 10000.0);
        assert_eq!(mm.realized_pnl(), 0.0);
    }

    #[test]
    fn test_quote() {
        let mut mm = MarketMaker::new(MarketMakerConfig::default(), 10000.0);
        mm.update_mid_price(100.0, 0.0);
        let q = mm.quote().unwrap();
        assert!(q.bid_price < q.ask_price);
        assert!(q.bid_price < 100.0);
        assert!(q.ask_price > 100.0);
        assert!(q.bid_size > 0.0);
        assert!(q.ask_size > 0.0);
    }

    #[test]
    fn test_buy_fill() {
        let mut mm = MarketMaker::new(MarketMakerConfig::default(), 10000.0);
        mm.update_mid_price(100.0, 0.0);
        mm.on_fill(Side::Buy, 99.0, 1.0, 1).unwrap();
        assert_eq!(mm.position(), 1.0);
        assert!(mm.cash() < 10000.0);
    }

    #[test]
    fn test_sell_fill() {
        let mut mm = MarketMaker::new(MarketMakerConfig::default(), 10000.0);
        mm.update_mid_price(100.0, 0.0);
        mm.on_fill(Side::Sell, 101.0, 1.0, 1).unwrap();
        assert_eq!(mm.position(), -1.0);
        assert!(mm.cash() > 10000.0);
    }

    #[test]
    fn test_round_trip_pnl() {
        let mut mm = MarketMaker::new(MarketMakerConfig::default(), 10000.0);
        mm.update_mid_price(100.0, 0.0);
        mm.on_fill(Side::Buy, 99.0, 1.0, 1).unwrap();
        mm.update_mid_price(101.0, 0.0);
        mm.on_fill(Side::Sell, 101.0, 1.0, 2).unwrap();
        assert!(mm.realized_pnl() > 0.0);
        assert_eq!(mm.position(), 0.0);
    }

    #[test]
    fn test_inventory_skew() {
        let mut mm = MarketMaker::new(MarketMakerConfig::default(), 10000.0);
        mm.update_mid_price(100.0, 0.0);
        // Go long
        mm.on_fill(Side::Buy, 99.0, 50.0, 1).unwrap();
        mm.update_mid_price(100.0, 0.0);
        let q = mm.quote().unwrap();
        // When long, bid should be lower (skew down)
        assert!(q.bid_price < 100.0);
    }

    #[test]
    fn test_max_position() {
        let mut mm = MarketMaker::new(MarketMakerConfig::default(), 10000.0);
        mm.update_mid_price(100.0, 0.0);
        mm.on_fill(Side::Buy, 99.0, 100.0, 1).unwrap();
        assert_eq!(mm.position(), 100.0);
        // Should fail to buy more
        assert!(mm.on_fill(Side::Buy, 99.0, 1.0, 2).is_err());
    }

    #[test]
    fn test_volatility_spread() {
        let mut mm = MarketMaker::new(MarketMakerConfig::default(), 10000.0);
        mm.update_mid_price(100.0, 0.0);
        let q_low = mm.quote().unwrap();
        mm.update_mid_price(100.0, 0.5);
        let q_high = mm.quote().unwrap();
        // Higher volatility = wider spread
        let spread_low = q_low.ask_price - q_low.bid_price;
        let spread_high = q_high.ask_price - q_high.bid_price;
        assert!(spread_high > spread_low);
    }

    #[test]
    fn test_zero_size_quote() {
        let mut mm = MarketMaker::new(MarketMakerConfig::default(), 10000.0);
        mm.update_mid_price(100.0, 0.0);
        mm.on_fill(Side::Buy, 99.0, 100.0, 1).unwrap();
        let q = mm.quote().unwrap();
        assert_eq!(q.bid_size, 0.0); // at max position, no more bids
        assert!(q.ask_size > 0.0);
    }

    #[test]
    fn test_invalid_inputs() {
        let mut mm = MarketMaker::new(MarketMakerConfig::default(), 10000.0);
        assert!(mm.quote().is_err()); // no mid price set
        mm.update_mid_price(100.0, 0.0);
        assert!(mm.on_fill(Side::Buy, 0.0, 1.0, 1).is_err());
        assert!(mm.on_fill(Side::Buy, 100.0, 0.0, 1).is_err());
    }

    #[test]
    fn test_unrealized_pnl() {
        let mut mm = MarketMaker::new(MarketMakerConfig::default(), 10000.0);
        mm.update_mid_price(100.0, 0.0);
        mm.on_fill(Side::Buy, 99.0, 1.0, 1).unwrap();
        mm.update_mid_price(105.0, 0.0);
        assert!(mm.unrealized_pnl() > 0.0);
        assert!(mm.total_pnl() > 0.0);
    }

    #[test]
    fn test_quote_count() {
        let mut mm = MarketMaker::new(MarketMakerConfig::default(), 10000.0);
        mm.update_mid_price(100.0, 0.0);
        mm.quote().unwrap();
        mm.quote().unwrap();
        assert_eq!(mm.quote_count(), 2);
    }
}
