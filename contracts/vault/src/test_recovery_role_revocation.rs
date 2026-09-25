//! Tests for Issue #1700: execute_recovery must revoke roles/delegations of
//! removed signers and assign Member role to newly added signers.

use crate::types::{
    InitConfig, RecoveryConfig, RetryConfig, Role, StakingConfig,
    ThresholdStrategy, VelocityConfig, VoteWeight,
};
use crate::{VaultDAO, VaultDAOClient};
use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

// ---------------------------------------------------------------------------
// Shared setup helper
// ---------------------------------------------------------------------------

struct Setup<'a> {
    env: Env,
    client: VaultDAOClient<'a>,
    admin: Address,
    /// Compromised signer that will be removed by recovery
    old_signer: Address,
    /// Guardian who triggers recovery
    guardian: Address,
    /// New signer that replaces old_signer
    new_signer: Address,
}

fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(VaultDAO, ());
    let client = VaultDAOClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let old_signer = Address::generate(&env);
    let guardian = Address::generate(&env);
    let new_signer = Address::generate(&env);

    let mut signers = Vec::new(&env);
    signers.push_back(admin.clone());
    signers.push_back(old_signer.clone());

    let mut guardians = Vec::new(&env);
    guardians.push_back(guardian.clone());

    let config = InitConfig {
        signers,
        threshold: 2,
        quorum: 0,
        quorum_percentage: 0,
        spending_limit: 1_000_000,
        daily_limit: 5_000_000,
        weekly_limit: 10_000_000,
        timelock_threshold: 0,
        timelock_delay: 0,
        velocity_limit: VelocityConfig {
            limit: 100_000,
            window: 3600,
            per_token_limit: 0,
        },
        threshold_strategy: ThresholdStrategy::Fixed,
        default_voting_deadline: 0,
        veto_addresses: Vec::new(&env),
        veto_window_ledgers: 0,
        whitelist_mode: false,
        grace_period_ledgers: 100,
        vote_weight: VoteWeight::Flat,
        high_impact_threshold: 70,
        admin_rotation_delay: 1440,
        proposal_id_prefix: 0,
        pre_execution_hooks: Vec::new(&env),
        post_execution_hooks: Vec::new(&env),
        retry_config: RetryConfig {
            enabled: false,
            max_retries: 0,
            initial_backoff_ledgers: 0,
            max_retry_delay: 0,
        },
        recovery_config: RecoveryConfig {
            guardians,
            // single guardian approval is enough
            threshold: 1,
            // no delay — execute immediately after approval
            delay: 0,
        },
        staking_config: StakingConfig::default(),
    };

    client.initialize(&admin, &config);

    Setup {
        env,
        client,
        admin,
        old_signer,
        guardian,
        new_signer,
    }
}

/// Drive a recovery proposal through propose → approve → execute.
/// Returns immediately after execution (delay == 0 in the test setup).
fn run_recovery(s: &Setup, new_signers: Vec<Address>, new_threshold: u32) {
    let proposal_id = s
        .client
        .initiate_recovery(&s.guardian, &new_signers, &new_threshold);

    s.client.approve_recovery(&s.guardian, &proposal_id);
    // execution_after == current_ledger + 0, so it is already reachable.
    s.client.execute_recovery(&proposal_id);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// After recovery, the old Admin-role signer must no longer hold Admin role.
#[test]
fn test_removed_signer_loses_role_after_recovery() {
    let s = setup();

    // Elevate old_signer to Admin so the bug is clearly visible.
    s.client.set_role(&s.admin, &s.old_signer, &Role::Admin);
    assert_eq!(s.client.get_role(&s.old_signer), Role::Admin);

    // Recovery replaces [admin, old_signer] with [admin, new_signer].
    let mut new_signers = Vec::new(&s.env);
    new_signers.push_back(s.admin.clone());
    new_signers.push_back(s.new_signer.clone());
    run_recovery(&s, new_signers, 2);

    // old_signer's explicit role entry must be gone; get_role returns default Member.
    assert_eq!(s.client.get_role(&s.old_signer), Role::Member);
}

/// The old signer must not appear in get_role_assignments after recovery.
#[test]
fn test_removed_signer_absent_from_role_assignments() {
    let s = setup();

    s.client
        .set_role(&s.admin, &s.old_signer, &Role::Treasurer);

    let mut new_signers = Vec::new(&s.env);
    new_signers.push_back(s.admin.clone());
    new_signers.push_back(s.new_signer.clone());
    run_recovery(&s, new_signers, 2);

    let assignments = s.client.get_role_assignments();
    for assignment in assignments.iter() {
        assert_ne!(
            assignment.addr, s.old_signer,
            "old_signer must not appear in role assignments after recovery"
        );
    }
}

/// Newly added signers must receive Role::Member after recovery.
#[test]
fn test_new_signer_receives_member_role_after_recovery() {
    let s = setup();

    // Before recovery new_signer has no entry at all.
    assert_eq!(s.client.get_role(&s.new_signer), Role::Member);

    let mut new_signers = Vec::new(&s.env);
    new_signers.push_back(s.admin.clone());
    new_signers.push_back(s.new_signer.clone());
    run_recovery(&s, new_signers, 2);

    // new_signer must now have an explicit Member role entry.
    assert_eq!(s.client.get_role(&s.new_signer), Role::Member);

    // And they should appear in the role index.
    let assignments = s.client.get_role_assignments();
    let found = assignments.iter().any(|a| a.addr == s.new_signer);
    assert!(found, "new_signer must appear in role assignments after recovery");
}

/// Signers retained across recovery must keep their roles unchanged.
#[test]
fn test_retained_signer_keeps_role_after_recovery() {
    let s = setup();

    // admin keeps their Admin role; they are in both old and new signer sets.
    assert_eq!(s.client.get_role(&s.admin), Role::Admin);

    let mut new_signers = Vec::new(&s.env);
    new_signers.push_back(s.admin.clone());
    new_signers.push_back(s.new_signer.clone());
    run_recovery(&s, new_signers, 2);

    assert_eq!(
        s.client.get_role(&s.admin),
        Role::Admin,
        "admin must keep their role across recovery"
    );
}

/// An active plain delegation held by a removed signer must be revoked.
#[test]
fn test_removed_signer_plain_delegation_revoked() {
    let s = setup();

    // old_signer delegates their voting power to admin.
    s.client.delegate_voting_power(
        &s.old_signer,
        &s.admin,
        // expiry far in the future
        &1_000_000u64,
    );

    // Verify the delegation chain is non-empty before recovery (old_signer → admin).
    let chain_before = s.client.get_delegation_chain(&s.old_signer).unwrap();
    assert!(
        !chain_before.is_empty(),
        "delegation chain should be non-empty before recovery"
    );

    let mut new_signers = Vec::new(&s.env);
    new_signers.push_back(s.admin.clone());
    new_signers.push_back(s.new_signer.clone());
    run_recovery(&s, new_signers, 2);

    // After recovery the delegation entry has been removed, so the chain is empty.
    let chain_after = s.client.get_delegation_chain(&s.old_signer).unwrap();
    assert!(
        chain_after.is_empty(),
        "removed signer's plain delegation must be revoked after recovery"
    );
}

/// All scoped delegations held by a removed signer must be deactivated.
#[test]
fn test_removed_signer_scoped_delegations_deactivated() {
    let s = setup();

    // old_signer creates a scoped delegation to admin.
    let mut allowed_ids: Vec<u64> = Vec::new(&s.env);
    allowed_ids.push_back(1u64);

    let scoped_id = s.client.create_scoped_delegation(
        &s.old_signer,
        &s.admin,
        &500_000i128,   // max_amount
        &100_000u32,    // expires_at_ledger (far future)
        &allowed_ids,
    );

    // Verify it is active.
    let sd_before = s.client.get_scoped_delegation(&scoped_id).unwrap();
    assert!(sd_before.is_active, "scoped delegation should be active before recovery");

    let mut new_signers = Vec::new(&s.env);
    new_signers.push_back(s.admin.clone());
    new_signers.push_back(s.new_signer.clone());
    run_recovery(&s, new_signers, 2);

    // After recovery the scoped delegation must be deactivated.
    let sd_after = s.client.get_scoped_delegation(&scoped_id).unwrap();
    assert!(
        !sd_after.is_active,
        "removed signer's scoped delegation must be deactivated after recovery"
    );
}

/// Full scenario: old Admin is replaced; they can no longer call an Admin-only
/// function (set_role). The new signer starts as Member and can be promoted.
#[test]
fn test_old_admin_loses_access_new_signer_gains_member() {
    let s = setup();

    // Give old_signer Admin role to match the bug scenario.
    s.client.set_role(&s.admin, &s.old_signer, &Role::Admin);

    let mut new_signers = Vec::new(&s.env);
    new_signers.push_back(s.admin.clone());
    new_signers.push_back(s.new_signer.clone());
    run_recovery(&s, new_signers, 2);

    // old_signer's role is now gone (reverted to default Member).
    assert_eq!(
        s.client.get_role(&s.old_signer),
        Role::Member,
        "compromised signer must lose Admin role after recovery"
    );

    // new_signer has Member role and is visible in the role index.
    assert_eq!(s.client.get_role(&s.new_signer), Role::Member);

    // The admin (retained) can now promote new_signer — proving normal RBAC
    // still works after recovery.
    s.client.set_role(&s.admin, &s.new_signer, &Role::Treasurer);
    assert_eq!(s.client.get_role(&s.new_signer), Role::Treasurer);
}

/// When a signer appears in both old and new lists (retained), their
/// delegation is NOT disturbed.
#[test]
fn test_retained_signer_delegation_preserved() {
    let s = setup();

    // old_signer delegates to admin (both are in the initial signer set).
    // We keep old_signer in the new set as well.
    s.client.delegate_voting_power(&s.old_signer, &s.admin, &1_000_000u64);

    let mut new_signers = Vec::new(&s.env);
    // Retain both original signers AND add new_signer (3-of-3 recovery).
    new_signers.push_back(s.admin.clone());
    new_signers.push_back(s.old_signer.clone());
    new_signers.push_back(s.new_signer.clone());
    run_recovery(&s, new_signers, 2);

    // old_signer is retained; their delegation chain must still be non-empty.
    let chain = s.client.get_delegation_chain(&s.old_signer).unwrap();
    assert!(
        !chain.is_empty(),
        "retained signer's delegation must survive recovery"
    );
}
