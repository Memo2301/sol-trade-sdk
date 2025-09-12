// Removed unused imports
use std::sync::Arc;

use crate::instruction::{
    bonk::BonkInstructionBuilder, pumpfun::PumpFunInstructionBuilder,
    pumpswap::PumpSwapInstructionBuilder, raydium_amm_v4::RaydiumAmmV4InstructionBuilder,
    raydium_cpmm::RaydiumCpmmInstructionBuilder, raydium_clmm::{RaydiumClmmInstructionBuilder, RaydiumClmmV2InstructionBuilder},
};

use super::core::{executor::GenericTradeExecutor, traits::TradeExecutor};

/// 支持的交易协议
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DexType {
    PumpFun,
    PumpSwap,
    Bonk,
    RaydiumCpmm,
    RaydiumClmm,
    RaydiumClmmV2,
    RaydiumAmmV4,
}

/// 交易工厂 - 用于创建不同协议的交易执行器
pub struct TradeFactory;

impl TradeFactory {
    /// 创建指定协议的交易执行器（零开销单例）
    pub fn create_executor(dex_type: DexType) -> Arc<dyn TradeExecutor> {
        match dex_type {
            DexType::PumpFun => Self::pumpfun_executor(),
            DexType::PumpSwap => Self::pumpswap_executor(),
            DexType::Bonk => Self::bonk_executor(),
            DexType::RaydiumCpmm => Self::raydium_cpmm_executor(),
            DexType::RaydiumClmm => Self::raydium_clmm_executor(),
            DexType::RaydiumClmmV2 => Self::raydium_clmm_v2_executor(),
            DexType::RaydiumAmmV4 => Self::raydium_amm_v4_executor(),
        }
    }

    // Static instances created at compile time - zero runtime overhead
    #[inline]
    fn pumpfun_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(PumpFunInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "PumpFun"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn pumpswap_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(PumpSwapInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "PumpSwap"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn bonk_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(BonkInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "Bonk"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn raydium_cpmm_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(RaydiumCpmmInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "RaydiumCpmm"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn raydium_clmm_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(RaydiumClmmInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "RaydiumClmm"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn raydium_clmm_v2_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(RaydiumClmmV2InstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "RaydiumClmmV2"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn raydium_amm_v4_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(RaydiumAmmV4InstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "RaydiumAmmV4"))
            });
        INSTANCE.clone()
    }
}
