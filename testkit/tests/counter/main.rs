// Copyright 2019 The Exonum Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use assert_matches::assert_matches;
use exonum::{
    api::{node::public::explorer::TransactionQuery, Error as ApiError},
    blockchain::TransactionErrorType as ErrorType,
    crypto::{self, CryptoHash, PublicKey},
    helpers::Height,
    messages::{self, RawTransaction, Signed},
};
use exonum_merkledb::HashTag;
use exonum_testkit::{txvec, ApiKind, ComparableSnapshot, TestKit, TestKitApi, TestKitBuilder};
use hex::FromHex;
use serde_json::{json, Value};

use crate::counter::{
    CounterSchema, CounterService, TransactionResponse, TxIncrement, TxReset, ADMIN_KEY,
};

mod counter;
mod proto;

fn init_testkit() -> (TestKit, TestKitApi) {
    let testkit = TestKit::for_service(CounterService);
    let api = testkit.api();
    (testkit, api)
}

fn inc_count(api: &TestKitApi, by: u64) -> Signed<RawTransaction> {
    let (pubkey, key) = crypto::gen_keypair();
    // Create a pre-signed transaction
    let tx = TxIncrement::sign(&pubkey, by, &key);

    let tx_info: TransactionResponse = api
        .public(ApiKind::Service("counter"))
        .query(&tx)
        .post("count")
        .unwrap();
    assert_eq!(tx_info.tx_hash, tx.hash());
    tx
}

#[test]
fn test_inc_count_create_block() {
    let (mut testkit, api) = init_testkit();
    let (pubkey, key) = crypto::gen_keypair();

    // Create a pre-signed transaction
    testkit.create_block_with_transaction(TxIncrement::sign(&pubkey, 5, &key));

    // Check that the user indeed is persisted by the service
    let counter: u64 = api
        .public(ApiKind::Service("counter"))
        .get("count")
        .unwrap();
    assert_eq!(counter, 5);

    testkit.create_block_with_transactions(txvec![
        TxIncrement::sign(&pubkey, 4, &key),
        TxIncrement::sign(&pubkey, 1, &key),
    ]);

    let counter: u64 = api
        .public(ApiKind::Service("counter"))
        .get("count")
        .unwrap();
    assert_eq!(counter, 10);
}

#[should_panic(expected = "Transaction is already committed")]
#[test]
fn test_inc_count_create_block_with_committed_transaction() {
    let (mut testkit, _) = init_testkit();
    let (pubkey, key) = crypto::gen_keypair();
    // Create a pre-signed transaction
    testkit.create_block_with_transaction(TxIncrement::sign(&pubkey, 5, &key));
    // Create another block with the same transaction
    testkit.create_block_with_transaction(TxIncrement::sign(&pubkey, 5, &key));
}

#[test]
fn test_inc_count_api() {
    let (mut testkit, api) = init_testkit();
    inc_count(&api, 5);
    testkit.create_block();

    // Check that the user indeed is persisted by the service
    let counter: u64 = api
        .public(ApiKind::Service("counter"))
        .get("count")
        .unwrap();
    assert_eq!(counter, 5);
}

#[test]
fn test_inc_count_with_multiple_transactions() {
    let (mut testkit, api) = init_testkit();

    for _ in 0..100 {
        inc_count(&api, 1);
        inc_count(&api, 2);
        inc_count(&api, 3);
        inc_count(&api, 4);

        testkit.create_block();
    }

    assert_eq!(testkit.height(), Height(100));
    let counter: u64 = api
        .public(ApiKind::Service("counter"))
        .get("count")
        .unwrap();
    assert_eq!(counter, 1_000);
}

#[test]
fn test_inc_count_with_manual_tx_control() {
    let (mut testkit, api) = init_testkit();
    let tx_a = inc_count(&api, 5);
    let tx_b = inc_count(&api, 3);

    // Empty block
    testkit.create_block_with_tx_hashes(&[]);
    let counter: u64 = api
        .public(ApiKind::Service("counter"))
        .get("count")
        .unwrap();
    assert_eq!(counter, 0);

    testkit.create_block_with_tx_hashes(&[tx_b.hash()]);
    let counter: u64 = api
        .public(ApiKind::Service("counter"))
        .get("count")
        .unwrap();
    assert_eq!(counter, 3);

    testkit.create_block_with_tx_hashes(&[tx_a.hash()]);
    let counter: u64 = api
        .public(ApiKind::Service("counter"))
        .get("count")
        .unwrap();
    assert_eq!(counter, 8);
}

#[test]
fn test_private_api() {
    let (mut testkit, api) = init_testkit();
    inc_count(&api, 5);
    inc_count(&api, 3);

    testkit.create_block();
    let counter: u64 = api
        .private(ApiKind::Service("counter"))
        .get("count")
        .unwrap();
    assert_eq!(counter, 8);

    let (pubkey, key) = crypto::gen_keypair_from_seed(
        &crypto::Seed::from_slice(&crypto::hash(b"correct horse battery staple")[..]).unwrap(),
    );
    assert_eq!(pubkey, PublicKey::from_hex(ADMIN_KEY).unwrap());

    let tx = TxReset::sign(&pubkey, &key);
    let tx_info: TransactionResponse = api
        .private(ApiKind::Service("counter"))
        .query(&tx)
        .post("reset")
        .unwrap();
    assert_eq!(tx_info.tx_hash, tx.hash());

    testkit.create_block();
    let counter: u64 = api
        .private(ApiKind::Service("counter"))
        .get("count")
        .unwrap();
    assert_eq!(counter, 0);
}

#[test]
fn test_probe() {
    let (mut testkit, api) = init_testkit();

    let tx = {
        let (pubkey, key) = crypto::gen_keypair();
        TxIncrement::sign(&pubkey, 5, &key)
    };

    let snapshot = testkit.probe(tx.clone());
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(5));
    // Verify that the patch has not been applied to the blockchain
    let counter: u64 = api
        .public(ApiKind::Service("counter"))
        .get("count")
        .unwrap();
    assert_eq!(counter, 0);

    let other_tx = {
        let (pubkey, key) = crypto::gen_keypair();
        TxIncrement::sign(&pubkey, 3, &key)
    };

    let snapshot = testkit.probe_all(txvec![tx.clone(), other_tx.clone()]);
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(8));

    // Posting a transaction is not enough to change the blockchain!
    let _: TransactionResponse = api
        .public(ApiKind::Service("counter"))
        .query(&tx)
        .post("count")
        .unwrap();
    let snapshot = testkit.probe(other_tx.clone());
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(3));
    testkit.create_block();
    let snapshot = testkit.probe(other_tx.clone());
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(8));
}

#[test]
fn test_duplicate_tx() {
    let (mut testkit, api) = init_testkit();

    let tx = inc_count(&api, 5);
    testkit.create_block();
    let _: TransactionResponse = api
        .public(ApiKind::Service("counter"))
        .query(&tx)
        .post("count")
        .unwrap();
    let _: TransactionResponse = api
        .public(ApiKind::Service("counter"))
        .query(&tx)
        .post("count")
        .unwrap();
    testkit.create_block();
    let counter: u64 = api
        .public(ApiKind::Service("counter"))
        .get("count")
        .unwrap();
    assert_eq!(counter, 5);
}

#[test]
fn test_probe_advanced() {
    let (mut testkit, api) = init_testkit();

    let tx = {
        let (pubkey, key) = crypto::gen_keypair();
        TxIncrement::sign(&pubkey, 6, &key)
    };
    let other_tx = {
        let (pubkey, key) = crypto::gen_keypair();
        TxIncrement::sign(&pubkey, 10, &key)
    };
    let admin_tx = {
        let (pubkey, key) = crypto::gen_keypair_from_seed(
            &crypto::Seed::from_slice(&crypto::hash(b"correct horse battery staple")[..]).unwrap(),
        );
        assert_eq!(pubkey, PublicKey::from_hex(ADMIN_KEY).unwrap());
        TxReset::sign(&pubkey, &key)
    };

    let snapshot = testkit.probe(tx.clone());
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(6));
    // Check that data is not persisted
    let snapshot = testkit.snapshot();
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), None);

    // Check dependency of the resulting snapshot on tx ordering
    let snapshot = testkit.probe_all(txvec![tx.clone(), admin_tx.clone()]);
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(0));
    let snapshot = testkit.probe_all(txvec![admin_tx.clone(), tx.clone()]);
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(6));
    // Check that data is (still) not persisted
    let snapshot = testkit.snapshot();
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), None);

    api.send(other_tx);
    testkit.create_block();
    let snapshot = testkit.snapshot();
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(10));

    let snapshot = testkit.probe(tx.clone());
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(16));
    // Check that data is not persisted
    let snapshot = testkit.snapshot();
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(10));

    // Check dependency of the resulting snapshot on tx ordering
    let snapshot = testkit.probe_all(txvec![tx.clone(), admin_tx.clone()]);
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(0));
    let snapshot = testkit.probe_all(txvec![admin_tx.clone(), tx.clone()]);
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(6));
    // Check that data is (still) not persisted
    let snapshot = testkit.snapshot();
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(10));
}

#[test]
fn test_probe_duplicate_tx() {
    //! Checks that committed transactions do not change the blockchain state when probed.

    let (mut testkit, api) = init_testkit();
    let tx = inc_count(&api, 5);

    let snapshot = testkit.probe(tx.clone());
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(5));

    testkit.create_block();

    let snapshot = testkit.probe(tx.clone());
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(5));

    // Check the mixed case, when some probed transactions are committed and some are not
    let other_tx = inc_count(&api, 7);
    let snapshot = testkit.probe_all(txvec![tx, other_tx]);
    let schema = CounterSchema::new(&snapshot);
    assert_eq!(schema.count(), Some(12));
}

#[test]
fn test_snapshot_comparison() {
    let (mut testkit, api) = init_testkit();

    let tx = {
        let (pubkey, key) = crypto::gen_keypair();
        TxIncrement::sign(&pubkey, 5, &key)
    };
    testkit
        .probe(tx.clone())
        .compare(testkit.snapshot())
        .map(CounterSchema::new)
        .map(CounterSchema::count)
        .assert_before("Counter does not exist", Option::is_none)
        .assert_after("Counter has been set", |&c| c == Some(5));

    api.send(tx);
    testkit.create_block();

    let other_tx = {
        let (pubkey, key) = crypto::gen_keypair();
        TxIncrement::sign(&pubkey, 3, &key)
    };
    testkit
        .probe(other_tx.clone())
        .compare(testkit.snapshot())
        .map(CounterSchema::new)
        .map(CounterSchema::count)
        .map(|&c| c.unwrap())
        .assert("Counter has increased", |&old, &new| new == old + 3);
}

#[test]
#[should_panic(expected = "Counter has increased")]
fn test_snapshot_comparison_panic() {
    let (mut testkit, api) = init_testkit();
    let increment_by = 5;
    let tx = {
        let (pubkey, key) = crypto::gen_keypair();
        TxIncrement::sign(&pubkey, increment_by, &key)
    };

    api.send(tx.clone());
    testkit.create_block();

    // The assertion fails because the transaction is already committed by now
    testkit
        .probe(tx.clone())
        .compare(testkit.snapshot())
        .map(CounterSchema::new)
        .map(CounterSchema::count)
        .map(|&c| c.unwrap())
        .assert("Counter has increased", |&old, &new| {
            new == old + increment_by
        });
}

fn create_sample_block(testkit: &mut TestKit) {
    let height = testkit.height().next().0;
    if height == 2 || height == 5 {
        let tx = {
            let (pubkey, key) = crypto::gen_keypair();
            TxIncrement::sign(&pubkey, height as u64, &key)
        };
        testkit.api().send(tx.clone());
    }
    testkit.create_block();
}

#[test]
fn test_explorer_blocks_basic() {
    use exonum::api::node::public::explorer::BlocksRange;
    use exonum::helpers::Height;

    let (mut testkit, api) = init_testkit();

    let BlocksRange { blocks, range } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=10")
        .unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block.height(), Height(0));
    assert_eq!(*blocks[0].block.prev_hash(), crypto::Hash::zero());
    assert_eq!(range.start, Height(0));
    assert_eq!(range.end, Height(1));

    // Check JSON presentation of the block
    let response: serde_json::Value = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=10")
        .unwrap();
    assert_eq!(
        response,
        json!({
            "range": { "start": 0, "end": 1 },
            "blocks": [{
                "proposer_id": 0,
                "height": 0,
                "tx_count": 0,
                "prev_hash": crypto::Hash::zero(),
                "tx_hash": HashTag::empty_list_hash(),
                "state_hash": blocks[0].block.state_hash(),
            }],
        })
    );

    // Check empty block creation
    testkit.create_block();

    let BlocksRange { blocks, range } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=10")
        .unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].block.height(), Height(1));
    assert_eq!(*blocks[0].block.prev_hash(), blocks[1].block.hash());
    assert_eq!(blocks[0].block.tx_count(), 0);
    assert_eq!(blocks[1].block.height(), Height(0));
    assert_eq!(*blocks[1].block.prev_hash(), crypto::Hash::default());
    assert_eq!(range.start, Height(0));
    assert_eq!(range.end, Height(2));

    // Check positioning of `precommits` and `block_time` within response.
    let response: serde_json::Value = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=10&earliest=1&add_precommits=true")
        .unwrap();
    let precommit = testkit.explorer().block(Height(1)).unwrap().precommits()[0].clone();
    assert_eq!(
        response,
        json!({
            "range": { "start": 1, "end": 2 },
            "blocks": [{
                "proposer_id": 0,
                "height": 1,
                "tx_count": 0,
                "prev_hash": blocks[1].block.hash(),
                "tx_hash": HashTag::empty_list_hash(),
                "state_hash": blocks[0].block.state_hash(),
                "precommits": [precommit],
            }],
        })
    );

    let response: serde_json::Value = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=10&earliest=1&add_blocks_time=true")
        .unwrap();
    assert_eq!(
        response,
        json!({
            "range": { "start": 1, "end": 2 },
            "blocks": [{
                "proposer_id": 0,
                "height": 1,
                "tx_count": 0,
                "prev_hash": blocks[1].block.hash(),
                "tx_hash": HashTag::empty_list_hash(),
                "state_hash": blocks[0].block.state_hash(),
                "time": precommit.time(),
            }],
        })
    );
}

#[test]
fn test_explorer_blocks_skip_empty_small() {
    use exonum::api::node::public::explorer::BlocksRange;
    use exonum::helpers::Height;

    let (mut testkit, api) = init_testkit();
    create_sample_block(&mut testkit);

    let BlocksRange { blocks, range } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=10&skip_empty_blocks=true")
        .unwrap();
    assert!(blocks.is_empty());
    assert_eq!(range.start, Height(0));
    assert_eq!(range.end, Height(2));

    create_sample_block(&mut testkit);

    let BlocksRange { blocks, range } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=10")
        .unwrap();
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].block.height(), Height(2));
    assert_eq!(*blocks[0].block.prev_hash(), blocks[1].block.hash());
    assert_eq!(blocks[0].block.tx_count(), 1);
    assert_eq!(range.start, Height(0));
    assert_eq!(range.end, Height(3));

    let BlocksRange { blocks, range } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=10&skip_empty_blocks=true")
        .unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block.height(), Height(2));
    assert_eq!(range.start, Height(0));
    assert_eq!(range.end, Height(3));

    create_sample_block(&mut testkit);
    create_sample_block(&mut testkit);

    let BlocksRange { blocks, range } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=10&skip_empty_blocks=true")
        .unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block.height(), Height(2));
    assert_eq!(range.start, Height(0));
    assert_eq!(range.end, Height(5));
}

#[test]
fn test_explorer_blocks_skip_empty() {
    use exonum::api::node::public::explorer::BlocksRange;
    use exonum::helpers::Height;

    let (mut testkit, api) = init_testkit();
    for _ in 0..5 {
        create_sample_block(&mut testkit);
    }

    let BlocksRange { blocks, range } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=1&skip_empty_blocks=true")
        .unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block.height(), Height(5));
    assert_eq!(range.start, Height(5));
    assert_eq!(range.end, Height(6));

    let BlocksRange { blocks, range } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=3&skip_empty_blocks=true")
        .unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].block.height(), Height(5));
    assert_eq!(blocks[1].block.height(), Height(2));
    assert_eq!(range.start, Height(0));
    assert_eq!(range.end, Height(6));
}

#[test]
fn test_explorer_blocks_bounds() {
    use exonum::api::node::public::explorer::BlocksRange;
    use exonum::helpers::Height;

    let (mut testkit, api) = init_testkit();
    for _ in 0..5 {
        create_sample_block(&mut testkit);
    }

    // Check `latest` param
    let BlocksRange { blocks, range } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=10&skip_empty_blocks=true&latest=4")
        .unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block.height(), Height(2));
    assert_eq!(range.start, Height(0));
    assert_eq!(range.end, Height(5));

    // Check `earliest` param
    let BlocksRange { blocks, range } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=10&earliest=3")
        .unwrap();
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0].block.height(), Height(5));
    assert_eq!(range.start, Height(3));
    assert_eq!(range.end, Height(6));

    // Check `earliest` & `latest`
    let BlocksRange { blocks, range } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=10&latest=4&earliest=3")
        .unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].block.height(), Height(4));
    assert_eq!(range.start, Height(3));
    assert_eq!(range.end, Height(5));

    // Check that `count` takes precedence over `earliest`.
    let BlocksRange { blocks, range } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=2&latest=4&earliest=1")
        .unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].block.height(), Height(4));
    assert_eq!(range.start, Height(3));
    assert_eq!(range.end, Height(5));

    // Check `latest` param isn't exceed the height.
    let BlocksRange { blocks, range } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=2&latest=5")
        .unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].block.height(), Height(5));
    assert_eq!(range.start, Height(4));
    assert_eq!(range.end, Height(6));

    // Check `latest` param is exceed the height.
    let result: Result<BlocksRange, ApiError> = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=2&latest=6");
    assert!(result.is_err());
}

#[test]
fn test_explorer_blocks_loaded_info() {
    use exonum::api::node::public::explorer::BlocksRange;
    use exonum::helpers::Height;

    let (mut testkit, api) = init_testkit();
    testkit.create_blocks_until(Height(6));

    let BlocksRange { blocks, .. } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=4")
        .unwrap();
    assert!(blocks
        .iter()
        .all(|info| info.time.is_none() && info.precommits.is_none()));

    let BlocksRange { blocks, .. } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=4&add_blocks_time=true")
        .unwrap();
    assert!(blocks
        .iter()
        .all(|info| info.time.is_some() && info.precommits.is_none()));

    let BlocksRange { blocks, .. } = api
        .public(ApiKind::Explorer)
        .get("v1/blocks?count=4&add_precommits=true")
        .unwrap();
    assert!(blocks
        .iter()
        .all(|info| info.time.is_none() && info.precommits.is_some()));
}

#[test]
fn test_explorer_single_block() {
    use exonum::explorer::BlockchainExplorer;
    use exonum::helpers::Height;
    use std::collections::HashSet;

    let mut testkit = TestKitBuilder::validator()
        .with_validators(4)
        .with_service(CounterService)
        .create();

    assert_eq!(testkit.majority_count(), 3);

    {
        let explorer = BlockchainExplorer::new(testkit.blockchain());
        let block = explorer.block(Height(0)).unwrap();
        assert_eq!(block.height(), Height(0));
        assert_eq!(*block.header().prev_hash(), crypto::Hash::default());
        assert_eq!(&*block.transaction_hashes(), &[]);
    }

    let tx = {
        let (pubkey, key) = crypto::gen_keypair();
        TxIncrement::sign(&pubkey, 5, &key)
    };
    testkit.api().send(tx.clone());
    testkit.create_block(); // height == 1

    {
        let explorer = BlockchainExplorer::new(testkit.blockchain());
        let block = explorer.block(Height(1)).unwrap();
        assert_eq!(block.height(), Height(1));
        assert_eq!(block.len(), 1);
        assert_eq!(*block.header().tx_hash(), HashTag::hash_list(&[tx.hash()]));
        assert_eq!(&*block.transaction_hashes(), &[tx.hash()]);

        let mut validators = HashSet::new();
        for precommit in block.precommits().iter() {
            assert_eq!(precommit.height(), Height(1));
            assert_eq!(*precommit.block_hash(), block.header().hash());
            let pk = testkit
                .network()
                .consensus_public_key_of(precommit.validator())
                .expect("Cannot find validator id");
            validators.insert(precommit.validator());
            assert_eq!(pk, &precommit.author())
        }

        assert!(validators.len() >= testkit.majority_count());
    }
}

#[test]
fn test_explorer_transaction_info() {
    use exonum::explorer::{BlockchainExplorer, TransactionInfo};
    use exonum::helpers::Height;

    let (mut testkit, api) = init_testkit();

    let tx = {
        let (pubkey, key) = crypto::gen_keypair();
        TxIncrement::sign(&pubkey, 5, &key)
    };

    let info = api
        .public(ApiKind::Explorer)
        .get::<Value>(&format!("v1/transactions?hash={}", &tx.hash().to_hex()))
        .unwrap_err();
    let error_body = json!({ "type": "unknown" });
    assert_matches!(
        info,
        ApiError::NotFound(ref body) if serde_json::from_str::<Value>(body).unwrap() == error_body
    );

    api.send(tx.clone());
    testkit.poll_events();

    let info: Value = api
        .public(ApiKind::Explorer)
        .get(&format!("v1/transactions?hash={}", &tx.hash().to_hex()))
        .unwrap();
    assert_eq!(
        info,
        json!({
            "type": "in-pool",
            "content": {
                "debug": TxIncrement::new(5),
                "message": messages::to_hex_string(&tx)
            },
        })
    );

    testkit.create_block();
    let info: TransactionInfo = api
        .public(ApiKind::Explorer)
        .get(&format!("v1/transactions?hash={}", &tx.hash().to_hex()))
        .unwrap();
    assert!(info.is_committed());
    let committed = info.as_committed().unwrap();
    assert_eq!(committed.location().block_height(), Height(1));
    assert!(committed.status().is_ok());

    let explorer = BlockchainExplorer::new(testkit.blockchain());
    let block = explorer.block(Height(1)).unwrap();
    assert!(committed
        .location_proof()
        .validate(
            *block.header().tx_hash(),
            u64::from(block.header().tx_count())
        )
        .is_ok());
}

#[test]
fn test_explorer_transaction_statuses() {
    use exonum::blockchain::TransactionResult;
    use exonum::explorer::TransactionInfo;

    let (mut testkit, api) = init_testkit();

    let tx = {
        let (pubkey, key) = crypto::gen_keypair();
        TxIncrement::sign(&pubkey, 5, &key)
    };
    let error_tx = {
        let (pubkey, key) = crypto::gen_keypair();
        TxIncrement::sign(&pubkey, 0, &key)
    };
    let panicking_tx = {
        let (pubkey, key) = crypto::gen_keypair();
        TxIncrement::sign(&pubkey, u64::max_value() - 3, &key)
    };

    let block = testkit.create_block_with_transactions(txvec![
        tx.clone(),
        error_tx.clone(),
        panicking_tx.clone(),
    ]);

    fn check_statuses(statuses: &[TransactionResult]) {
        assert!(statuses[0].0.is_ok());
        assert_matches!(
            statuses[1],
            TransactionResult(Err(ref err)) if err.error_type() == ErrorType::Code(0)
                && err.description() == Some("Adding zero does nothing!")
        );
        assert_matches!(
            statuses[2],
            TransactionResult(Err(ref err)) if err.error_type() == ErrorType::Panic
                && err.description() == Some("attempt to add with overflow")
        );
    }

    // Check statuses retrieved from a block.
    let statuses: Vec<_> = block
        .transactions
        .iter()
        .map(|tx| TransactionResult(tx.status().map_err(Clone::clone)))
        .collect();
    check_statuses(&statuses);

    // Now, the same statuses retrieved via explorer web API.
    let statuses: Vec<_> = [tx.hash(), error_tx.hash(), panicking_tx.hash()]
        .iter()
        .map(|hash| {
            let info: TransactionInfo = api
                .public(ApiKind::Explorer)
                .query(&TransactionQuery::new(*hash))
                .get("v1/transactions")
                .unwrap();
            TransactionResult(info.as_committed().unwrap().status().map_err(Clone::clone))
        })
        .collect();
    check_statuses(&statuses);
}

// Make sure that boxed transaction can be used in the `TestKitApi::send`.
#[test]
fn test_boxed_tx() {
    let (mut testkit, api) = init_testkit();

    let tx = {
        let (pubkey, key) = crypto::gen_keypair();
        TxIncrement::sign(&pubkey, 5, &key)
    };

    api.send(tx);
    let block = testkit.create_block();
    assert_eq!(block.len(), 1);
    assert_eq!(
        block[0].content().message().service_id(),
        counter::SERVICE_ID
    );
}

#[test]
fn test_custom_headers_handling() {
    use reqwest::header::AUTHORIZATION;

    let (mut testkit, api) = init_testkit();
    testkit.create_block_with_transaction({
        let (pubkey, key) = crypto::gen_keypair();
        TxIncrement::sign(&pubkey, 5, &key)
    });

    let error = api
        .public(ApiKind::Service("counter"))
        .get::<u64>("v1/counter-with-auth")
        .unwrap_err();
    assert_matches!(error, ApiError::Unauthorized);

    let error = api
        .public(ApiKind::Service("counter"))
        .with(|req| req.header(AUTHORIZATION, "None"))
        .get::<u64>("v1/counter-with-auth")
        .unwrap_err();
    assert_matches!(error, ApiError::Unauthorized);

    let error = api
        .public(ApiKind::Service("counter"))
        .with(|req| req.header(AUTHORIZATION, "Bearer foobar"))
        .get::<u64>("v1/counter-with-auth")
        .unwrap_err();
    assert_matches!(error, ApiError::Unauthorized);

    let counter: u64 = api
        .public(ApiKind::Service("counter"))
        .with(|req| req.header(AUTHORIZATION, "Bearer SUPER_SECRET_111"))
        .get("v1/counter-with-auth")
        .unwrap();
    assert_eq!(counter, 5);
}
