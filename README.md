# Coin Comic Tales

A Bitcoin regtest API server that demonstrates basic wallet operations and transaction handling.

## Prerequisites

1. Bitcoin Core running in regtest mode with the following configuration:

```
regtest=1
server=1
rpcuser=alice
rpcpassword=password
```

you can use ./docker-compose.yaml or download image `btcpayserver/bitcoin:29.0` and run the container manually
`docker pull btcpayserver/bitcoin:29.0`

2. Create a `.env` file in the project root with:

```
user=alice
password=password
rpc_url=http://localhost:18443
```

## Running the Server

### Using Docker

1. build:
`docker build -t coin-comic-tales-rs .`

2. run:
`docker run -p 8021:8021 \
    -e user=alice \
    -e password=password \
    -e rpc_url=http://localhost:18443 \
    -e server_url=http://localhost:8021 \
    coin-comic-tales-rs`

### Using Rust

1. Start Bitcoin Core in regtest mode
2. Start the API server:

```bash
cargo run
```

The server will start at http://127.0.0.1:8021

## API Usage Guide

### 1. Create Wallets

Create both Miner and Trader wallets:

```bash
# Create Miner wallet
curl -X POST http://127.0.0.1:8021/wallet \
  -H "Content-Type: application/json" \
  -d '{"name": "Miner"}'

# Create Trader wallet
curl -X POST http://127.0.0.1:8021/wallet \
  -H "Content-Type: application/json" \
  -d '{"name": "Trader"}'
```

### 2. Get Mining Address and Generate Initial Blocks

First, you need to get a mining address from the Miner wallet. The API will automatically use Bech32 address type.
Then mine the initial blocks (101) to get spendable coins:

```bash
# Get new address from Miner wallet and save it for later use
MINER_ADDRESS=$(curl -X POST http://127.0.0.1:8021/address \
  -H "Content-Type: application/json" \
  -d "{
    \"wallet_name\": \"Miner\",
    \"name\": \"My Reward\"
  }" | tr -d '"')

# Mine initial 101 blocks to make coins spendable
curl -X POST http://127.0.0.1:8021/mine \
  -H "Content-Type: application/json" \
  -d "{
    \"wallet_name\": \"Miner\",
    \"address\": \"$MINER_ADDRESS\",
    \"blocks\": 101
  }"

# Check the balance
curl -X GET http://127.0.0.1:8021/wallet/Miner/balance
```

### 3. Get Trading Address and Send Bitcoin

Get a new address from the Trader wallet and send BTC from Miner to Trader:

```bash
# Get new address from Trader wallet
TRADER_ADDRESS=$(curl -X POST http://127.0.0.1:8021/address \
  -H "Content-Type: application/json" \
  -d "{
    \"wallet_name\": \"Trader\",
    \"name\": \"My Savings\"
  }" | tr -d '"')

# Send 20 BTC from Miner to Trader
TXID=$(curl -X POST http://127.0.0.1:8021/send \
  -H "Content-Type: application/json" \
  -d '{"from_wallet": "Miner", "to_address": "'$TRADER_ADDRESS'", 
"amount": 20.0, "message": "I will send you some BTC for trading!"}' | tr -d '"')
```

### 4. Check Transaction in Mempool

Check the transaction details in the mempool:

```bash
# Check mempool entry
curl -X GET "http://127.0.0.1:8021/mempool/$TXID" | jq
```

### 5. Mine Block to Confirm Transaction

Mine one more block to confirm the transaction:

```bash
# Mine 1 block to confirm the transaction
curl -X POST http://127.0.0.1:8021/mine \
  -H "Content-Type: application/json" \
  -d "{
    \"wallet_name\": \"Miner\",
    \"address\": \"$MINER_ADDRESS\",
    \"blocks\": 1
  }"
```

### 6. Check Transaction Details

Finally, check the confirmed transaction details:

```bash
# Get transaction details
curl -X GET "http://127.0.0.1:8021/tx/$TXID" | jq 
```

## Expected Results

After following these steps:

1. The Miner wallet will have mined 102 blocks total (101 initial + 1 confirmation)
2. 20 BTC will have been transferred from Miner to Trader
3. The transaction will be confirmed in the blockchain
4. Transaction details will be available via the API

The output file `out.txt` will be created with the transaction details in the following format:

```
<transaction_id>
<miner_input_address>
<miner_input_amount>
<trader_output_address>
<trader_output_amount>
<miner_change_address>
<miner_change_amount>
<fee>
<block_height>
<confirmation_block_hash>
```
