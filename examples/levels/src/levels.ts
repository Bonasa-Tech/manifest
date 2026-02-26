import {
    ManifestClient, WrapperPlaceOrderParamsExternal,
} from '@cks-systems/manifest-sdk';
import { OrderType } from '@cks-systems/manifest-sdk/client/ts/src/manifest/types';
import { WrapperCancelOrderParams } from '@cks-systems/manifest-sdk/client/ts/src/wrapper';
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
    INTERVAL_SECS, 
    MAX_NUM_CANCEL_IX,
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

let previousLevel: number | null = null;

/**
 * Generates orders based on the current market position and level changes.
 * Maintains consistent order ID convention where:
 * - Ask orders use level number as ID (e.g., level 1 ask = ID 1)
 * - Bid orders use level number + 100 as ID (e.g., level 1 bid = ID 101)
 */
function generateOrders(
    levels: BondingLevel[],
    currentLevel: number
): {
    orderParams: WrapperPlaceOrderParamsExternal[],
    cancelParams: WrapperCancelOrderParams[]
} {
    let orderParams: WrapperPlaceOrderParamsExternal[] = [];
    let cancelParams: WrapperCancelOrderParams[] = [];

    // Initial setup - place all orders
    if (previousLevel === null) {
        console.log('Initializing Startup Orders')
        let orderParams: WrapperPlaceOrderParamsExternal[] = [];
        let cancelParams: WrapperCancelOrderParams[] = [];
        // Place buy orders for cleared levels
        for (let i = 1; i < currentLevel; i++) {
            const level = levels[i -1];
            orderParams.push({
                numBaseTokens: level.quantity,
                tokenPrice: level.price,
                isBid: true,
                lastValidSlot: 0,
                orderType: OrderType.PostOnly,
                clientOrderId: i + 100
            });
            cancelParams.push({clientOrderId: i + 100})
        }
        for (let i = currentLevel; i <= levels.length; i++) {
            const level = levels[i - 1];
            orderParams.push({
                numBaseTokens: level.quantity,
                tokenPrice: level.price,
                isBid: false,
                lastValidSlot: 0,
                orderType: OrderType.PostOnly,
                clientOrderId: i
            });
            cancelParams.push({clientOrderId: i })
        }
        previousLevel = currentLevel;
        return { orderParams, cancelParams };
    }

    // Handle level change
    console.log('Level change! Previous', previousLevel, ' Current', currentLevel);
    const isLevelIncreasing = currentLevel > previousLevel;

    if (isLevelIncreasing) {
        // Moving up: Replace intermediate levels with bids
        // Skip the lowest active bid level and highest level (active on ask side)
        for (let i = previousLevel; i < currentLevel; i++) {
            const level = levels[i -1];
            const bidOrderId = i + 100; // Add 100 to level number for bids
            orderParams.push({
                numBaseTokens: level.quantity,
                tokenPrice: level.price,
                isBid: true,
                lastValidSlot: 0,
                orderType: OrderType.Limit,
                clientOrderId: bidOrderId
            });
            // Cancel the corresponding ask order
            cancelParams.push({clientOrderId: i});
        }
    } else {
        // Moving down: Replace intermediate levels with asks
        // Skip the highest active ask level and lowest level (active on bid side)
        for (let i = currentLevel; i < previousLevel; i++) {
            const level = levels[i - 1];
            const askOrderId = i; // Use level number directly for asks
            orderParams.push({
                numBaseTokens: level.quantity,
                tokenPrice: level.price,
                isBid: false,
                lastValidSlot: 0,
                orderType: OrderType.Limit,
                clientOrderId: askOrderId
            });
            // Cancel the corresponding bid order if it exists
            cancelParams.push({clientOrderId: i + 100});
        }
    }

    previousLevel = currentLevel;
    return { orderParams, cancelParams };
}

/**
 * Gets the current level based on total tokens sold.
 * Levels are 1-based to match the order ID convention.
 */
function getCurrentLevel(
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

async function sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
}

async function sendOrderAndCancelBatches(
    connection: Connection,
    mfxClient: ManifestClient,
    owner: Keypair,
    orders: WrapperPlaceOrderParamsExternal[],
    cancels: WrapperCancelOrderParams[],
): Promise<string[]> {
    const signatures: string[] = [];
    let remainingOrders = [...orders];
    let remainingCancels = [...cancels];

    console.log(`Processing ${orders.length} orders and ${cancels.length} cancellations`);

    while (remainingOrders.length > 0 || remainingCancels.length > 0) {
        // Calculate how many operations we can fit in this batch
        const cancelBatchSize = Math.min(remainingCancels.length, MAX_NUM_CANCEL_IX);
        const orderBatchSize = Math.min(
            remainingOrders.length, 
            MAX_NUM_ORDER_IX - cancelBatchSize
        );

        const batchOrders = remainingOrders.slice(0, orderBatchSize);
        const batchCancels = remainingCancels.slice(0, cancelBatchSize);

        try {
            const batchOrderIx = mfxClient.batchUpdateIx(
                batchOrders,
                batchCancels,
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
                `Batch processed ${batchCancels.length} cancels, ${batchOrders.length} orders,`,
                `signature: ${signature}`
            );

            // Remove processed items from remaining arrays
            remainingOrders = remainingOrders.slice(orderBatchSize);
            remainingCancels = remainingCancels.slice(cancelBatchSize);
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
    console.log('Levels running. Market', mfxClient.market.address.toBase58(), 'Base', mfxClient.market.baseMint().toBase58())

    // Generate bonding levels once at startup
    const bondingLevels = generateBondingLevels(TOKEN_SUPPLY, NUM_LEVELS);
    
    while (true) {
        try {
            await mfxClient.reload();
            const { 
                baseWithdrawableBalanceTokens,
                baseOpenOrdersBalanceTokens,
            } = mfxClient.market.getBalances(owner.publicKey);
            
            const totalTokensSold = TOKEN_SUPPLY - baseWithdrawableBalanceTokens - baseOpenOrdersBalanceTokens;
            console.log('Balances OO', baseOpenOrdersBalanceTokens, 'Withdrawable', baseWithdrawableBalanceTokens);
            const currentLevel = getCurrentLevel(totalTokensSold, bondingLevels);
            console.log('Market Status:', {
                currentLevel: currentLevel,
                totalTokensSold: totalTokensSold,
                currentPrice: bondingLevels[currentLevel - 1].price,
                remainingLevels: bondingLevels.length - currentLevel
            });
            console.log('Bid', bondingLevels[currentLevel - 2], 'Ask', bondingLevels[currentLevel - 1]);
            if (currentLevel === previousLevel) {
                console.log('No level change.')
            } else {
                const { orderParams, cancelParams } = generateOrders(bondingLevels, currentLevel);
                
                const buyOrders = orderParams.filter(order => order.isBid);
                const sellOrders = orderParams.filter(order => !order.isBid);
                console.log('Order Summary:', {
                    buyOrders: buyOrders.length,
                    sellOrders: sellOrders.length,
                });
            // Assumes tokens have been deposited to Manifest
            await sendOrderAndCancelBatches(
                connection,
                mfxClient,
                owner,
                orderParams,
                cancelParams,
            );
        }
            console.log('Refresh in', INTERVAL_SECS, 'secs')
            await sleep(INTERVAL_SECS * 1000);
        } catch (error) {
            console.error("Error:", error);
            await sleep(INTERVAL_SECS * 1000);
        }
    }
}

main().catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
});
