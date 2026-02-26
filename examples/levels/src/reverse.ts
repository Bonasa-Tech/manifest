import {
    ManifestClient,
    WrapperPlaceOrderReverseParamsExternal,
} from '@cks-systems/manifest-sdk';
import { OrderType } from '@cks-systems/manifest-sdk/client/ts/src/manifest/types';
import { 
    Connection, 
    Keypair, 
    Commitment, 
    sendAndConfirmTransaction, 
    Transaction, 
    PublicKey
} from "@solana/web3.js";
import * as dotenv from "dotenv";
import bs58 from "bs58";
import { 
    INITIAL_FDV,
    INITIAL_QTY_RATIO, 
    INITIAL_QTY_REDUCTION,
    QTY_DECR_REDUCTION,
    INITIAL_PRICE_INCR,
    PRICE_INCR_REDUCTION,
    TOKEN_SUPPLY,
    NUM_LEVELS,
    MAX_NUM_ORDER_IX
} from './constants';

dotenv.config();

// Envs
const PRIVATE_KEY = process.env.PRIVATE_KEY;
const RPC_URL = process.env.RPC_URL;
const MFX_MKT = process.env.MFX_MKT;

interface BondingLevel {
    price: number;
    quantity: number;
    level: number;
}

/**
 * Generates order levels similar to a bonding curve from AMMs 
 * Returns the bonding levels corresponding prices and quantities.
 * Price increases and quantity decreases are reduced at each level.
 */
function generateBondingLevels(
    totalSupply: number = 1_000_000_000,
    numLevels: number = 100,
): BondingLevel[] {
    const levels: BondingLevel[] = [];
    const initialPrice = INITIAL_FDV / totalSupply;
    
    let currentQuantity = totalSupply * INITIAL_QTY_RATIO;
    let currentPrice = initialPrice;
    
    for (let level = 0; level < numLevels; level++) {
        levels.push({
            price: currentPrice,
            quantity: currentQuantity,
            level: level + 1,
        });
        const currentPriceIncrease = Math.max(INITIAL_PRICE_INCR - (level * PRICE_INCR_REDUCTION), 0);
        const currentQuantityReduction = Math.min(0, INITIAL_QTY_REDUCTION - (level * QTY_DECR_REDUCTION));
        
        currentQuantity *= (1 + currentQuantityReduction);
        currentPrice *= (1 + currentPriceIncrease);
    }

    console.log('Price Range:', {
        initialPrice,
        finalPrice: levels[levels.length - 1].price,
        priceMultiple: levels[levels.length - 1].price / initialPrice,
        numLevels: levels.length
    });
    
    return levels;
}

/**
 * Gets the current level based on total tokens sold only.
 * This can potentially be innaccurate if a spread is configured.
 * Levels are 1-based to match the order ID convention.
 */
function getCurrentLevelBasedOnPositionOnly(
    totalTokensSold: number,
    levels: BondingLevel[]
): number {
    let accumulatedTokens = 0;
    
    for (let i = 0; i < levels.length; i++) {
        accumulatedTokens += levels[i].quantity;
        if (accumulatedTokens >= totalTokensSold) {
            return i + 1;
        }
    }
    
    return levels.length;
}

/**
 * Generates initial orders for the bonding curve using reverse orders.
 * Each order will automatically flip between bid/ask after being filled.
 */
function generateInitialOrders(
    levels: BondingLevel[],
    currentLevel: number
): WrapperPlaceOrderReverseParamsExternal[] {
    const orders: WrapperPlaceOrderReverseParamsExternal[] = [];

    // Place ask orders to replicate bonding curve
    for (let i = currentLevel; i <= levels.length; i++) {
        const level = levels[i - 1];
        orders.push({
            numBaseTokens: level.quantity,
            tokenPrice: level.price,
            isBid: false,
            spreadBps: 100, // Default 1%
            orderType: OrderType.Reverse,  // Use reverse orders
            clientOrderId: i
        });
    }

    return orders;
}

async function sendOrderBatches(
    connection: Connection,
    mfxClient: ManifestClient,
    owner: Keypair,
    orders: WrapperPlaceOrderReverseParamsExternal[],
): Promise<string[]> {
    const signatures: string[] = [];
    let remainingOrders = [...orders];

    console.log(`Processing ${orders.length} orders`);

    while (remainingOrders.length > 0) {
        const batchSize = Math.min(remainingOrders.length, MAX_NUM_ORDER_IX);
        const batchOrders = remainingOrders.slice(0, batchSize);

        try {
            const batchOrderIx = mfxClient.batchUpdateIx(
                batchOrders,
                [], // No cancellations needed with reverse orders
                false
            );

            const tx = new Transaction();
            tx.add(batchOrderIx);
            
            const signature = await sendAndConfirmTransaction(
                connection,
                tx,
                [owner],
                { commitment: 'confirmed' }
            );
            
            signatures.push(signature);
            console.log(
                `Batch processed ${batchOrders.length} orders,`,
                `signature: ${signature}`
            );

            remainingOrders = remainingOrders.slice(batchSize);
        } catch (error) {
            console.error("Error processing batch:", error);
            throw error;
        }
    }

    return signatures;
}

async function main() {
    const connConfig = {
        commitment: 'processed' as Commitment,
        fetch: fetch as any,
    };

    if (!RPC_URL || !PRIVATE_KEY || !MFX_MKT) {
        console.log('Set envs!');
        return;
    }

    const connection = new Connection(RPC_URL, connConfig);
    const owner = Keypair.fromSecretKey(bs58.decode(PRIVATE_KEY));

    const mfxClient: ManifestClient = await ManifestClient.getClientForMarket(
        connection,
        new PublicKey(MFX_MKT),
        owner,
    );
    console.log('Levels running with Reverse orders. Market', mfxClient.market.address.toBase58(), 'Base', mfxClient.market.baseMint().toBase58())

    // Generate bonding levels
    const bondingLevels = generateBondingLevels(TOKEN_SUPPLY, NUM_LEVELS);
    
    // Place initial orders
    await mfxClient.reload();
    const { 
        baseWithdrawableBalanceTokens,
        baseOpenOrdersBalanceTokens,
    } = mfxClient.market.getBalances(owner.publicKey);
    
    const totalTokensSold = TOKEN_SUPPLY - baseWithdrawableBalanceTokens - baseOpenOrdersBalanceTokens;
    const currentLevel = getCurrentLevelBasedOnPositionOnly(totalTokensSold, bondingLevels);
    
    console.log('Market Status:', {
        currentLevel,
        totalTokensSold,
        withdrawableBalance: baseWithdrawableBalanceTokens,
        openOrdersBalance: baseOpenOrdersBalanceTokens,
        currentPrice: bondingLevels[currentLevel - 1].price,
        remainingLevels: bondingLevels.length - currentLevel
    });

    // Place all orders as reverse orders
    const orders = generateInitialOrders(bondingLevels, currentLevel);
    await sendOrderBatches(connection, mfxClient, owner, orders);
    console.log('Levels Complete!')
}

main().catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
});