use infinite_bounties_client::{
    InfiniteBountiesClient, InfiniteBountiesClientCtors, bounty_board::BountyBoard,
};
use sails_rs::{client::*, gtest::*, prelude::ActorId};

const ADMIN_ID: u64 = 42;
const FEE: u128 = 1_000_000_000_000;

#[tokio::test]
async fn test_get_config_after_deploy() {
    let system = System::new();
    let admin: ActorId = ADMIN_ID.into();
    system.mint_to(ADMIN_ID, 100_000_000_000_000);

    let env = GtestEnv::new(system, admin);
    let code_id = env.system().submit_code(infinite_bounties::WASM_BINARY);
    let program = env
        .deploy(code_id, b"salt".to_vec())
        .new(admin, FEE)
        .await
        .unwrap();

    let svc = program.bounty_board();
    let config = svc.get_config().await.unwrap();

    assert_eq!(config.admin, admin);
    assert_eq!(config.fee, FEE);
    assert_eq!(config.bounty_count, 0);
}

#[tokio::test]
async fn test_post_bounty_creates_bounty() {
    let system = System::new();
    let admin: ActorId = ADMIN_ID.into();
    system.mint_to(ADMIN_ID, 100_000_000_000_000);

    let env = GtestEnv::new(system, admin);
    let code_id = env.system().submit_code(infinite_bounties::WASM_BINARY);
    let program = env
        .deploy(code_id, b"salt".to_vec())
        .new(admin, FEE)
        .await
        .unwrap();

    let mut svc = program.bounty_board();

    let value: u128 = FEE + 500_000_000_000; // 1.5 TVARA
    let bounty_id = svc
        .post_bounty("Test".into(), "https://x.com".into())
        .with_value(value)
        .await
        .unwrap();

    assert_eq!(bounty_id, 1);

    let config = svc.get_config().await.unwrap();
    assert_eq!(config.bounty_count, 1);

    let bounty = svc.get_bounty(bounty_id).await.unwrap();
    assert!(bounty.is_some());
}

#[tokio::test]
async fn test_set_fee_updates_config() {
    let system = System::new();
    let admin: ActorId = ADMIN_ID.into();
    system.mint_to(ADMIN_ID, 100_000_000_000_000);

    let env = GtestEnv::new(system, admin);
    let code_id = env.system().submit_code(infinite_bounties::WASM_BINARY);
    let program = env
        .deploy(code_id, b"salt".to_vec())
        .new(admin, FEE)
        .await
        .unwrap();

    let mut svc = program.bounty_board();

    let new_fee: u128 = 5_000_000_000_000;
    svc.set_fee(new_fee).await.unwrap();

    let config = svc.get_config().await.unwrap();
    assert_eq!(config.fee, new_fee);
}
