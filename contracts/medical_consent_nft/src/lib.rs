#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, String, Vec,
};

// Storage keys

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    Issuers,
    TokenCounter,
    TokenOwner(u64),
    TokenMetadata(u64),
    TokenRevoked(u64),
    OwnerTokens(Address),
    ConsentHistory(u64),
}

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractError {
    NotAuthorized = 1,
    TokenNotFound = 2,
    ConsentRevoked = 3,
    AlreadyInitialized = 4,
    NotTokenOwner = 5,
}

// Consent metadata structure
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsentMetadata {
    pub metadata_uri: String,  // IPFS hash or secure storage pointer
    pub consent_type: String,  // Type of consent (treatment, research, etc.)
    pub issued_timestamp: u64, // When consent was issued
    pub expiry_timestamp: u64, // When consent expires (0 = no expiry)
    pub issuer: Address,       // Who issued the consent
    pub version: u32,          // Metadata version for updates
}

// Consent history entry for audit trail
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsentHistoryEntry {
    pub action: String, // "issued", "updated", "revoked"
    pub timestamp: u64,
    pub actor: Address,
    pub metadata_uri: String,
}

#[contract]
pub struct PatientConsentToken;

#[contractimpl]
impl PatientConsentToken {
    /// Initialize the contract with an admin
    pub fn initialize(env: Env, admin: Address) -> Result<(), ContractError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(ContractError::AlreadyInitialized);
        }

        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::TokenCounter, &0u64);

        // Initialize empty issuers list
        let issuers: Vec<Address> = Vec::new(&env);
        env.storage().instance().set(&DataKey::Issuers, &issuers);
        Ok(())
    }

    /// Add an authorized issuer (clinic/healthcare provider)
    pub fn add_issuer(env: Env, issuer: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        let mut issuers: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::Issuers)
            .unwrap_or(Vec::new(&env));

        issuers.push_back(issuer);
        env.storage().instance().set(&DataKey::Issuers, &issuers);
    }

    /// Remove an authorized issuer
    pub fn remove_issuer(env: Env, issuer: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        let issuers: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::Issuers)
            .expect("No issuers found");

        let mut new_issuers = Vec::new(&env);
        for i in 0..issuers.len() {
            let current = issuers.get(i).unwrap();
            if current != issuer {
                new_issuers.push_back(current);
            }
        }

        env.storage()
            .instance()
            .set(&DataKey::Issuers, &new_issuers);
    }

    /// Check if address is an authorized issuer
    pub fn is_issuer(env: Env, address: Address) -> bool {
        let issuers: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::Issuers)
            .unwrap_or(Vec::new(&env));

        for i in 0..issuers.len() {
            if issuers.get(i).unwrap() == address {
                return true;
            }
        }
        false
    }

    /// Mint a new consent token
    pub fn mint_consent(
        env: Env,
        to: Address,
        metadata_uri: String,
        consent_type: String,
        expiry_timestamp: u64,
    ) -> Result<u64, ContractError> {
        // Verify caller is authorized issuer
        let caller = to.clone();
        if !Self::is_issuer(env.clone(), caller.clone()) {
            return Err(ContractError::NotAuthorized);
        }

        to.require_auth();

        // Get and increment token counter
        let token_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::TokenCounter)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TokenCounter, &(token_id + 1));

        // Create consent metadata
        let metadata = ConsentMetadata {
            metadata_uri: metadata_uri.clone(),
            consent_type: consent_type.clone(),
            issued_timestamp: env.ledger().timestamp(),
            expiry_timestamp,
            issuer: caller.clone(),
            version: 1,
        };

        // Store token data
        env.storage()
            .instance()
            .set(&DataKey::TokenOwner(token_id), &to);
        env.storage()
            .instance()
            .set(&DataKey::TokenMetadata(token_id), &metadata);
        env.storage()
            .instance()
            .set(&DataKey::TokenRevoked(token_id), &false);

        // Add to owner's token list
        let owner_key = DataKey::OwnerTokens(to.clone());
        let mut owner_tokens: Vec<u64> = env
            .storage()
            .instance()
            .get(&owner_key)
            .unwrap_or(Vec::new(&env));
        owner_tokens.push_back(token_id);
        env.storage().instance().set(&owner_key, &owner_tokens);

        // Initialize consent history
        let history_entry = ConsentHistoryEntry {
            action: String::from_str(&env, "issued"),
            timestamp: env.ledger().timestamp(),
            actor: caller.clone(),
            metadata_uri: metadata_uri.clone(),
        };
        let mut history = Vec::new(&env);
        history.push_back(history_entry);
        env.storage()
            .instance()
            .set(&DataKey::ConsentHistory(token_id), &history);

        // Emit event
        env.events().publish(
            (symbol_short!("consent"), symbol_short!("issued")),
            (token_id, to, consent_type, metadata_uri),
        );

        Ok(token_id)
    }

    /// Update consent metadata (creates new version)
    pub fn update_consent(
        env: Env,
        token_id: u64,
        new_metadata_uri: String,
    ) -> Result<(), ContractError> {
        // Verify token exists and is not revoked
        let owner: Address = env
            .storage()
            .instance()
            .get(&DataKey::TokenOwner(token_id))
            .expect("Token does not exist");

        let is_revoked: bool = env
            .storage()
            .instance()
            .get(&DataKey::TokenRevoked(token_id))
            .unwrap_or(false);

        if is_revoked {
            return Err(ContractError::ConsentRevoked);
        }

        // Verify caller is issuer or owner
        owner.require_auth();

        // Get and update metadata
        let mut metadata: ConsentMetadata = env
            .storage()
            .instance()
            .get(&DataKey::TokenMetadata(token_id))
            .expect("Metadata not found");

        metadata.metadata_uri = new_metadata_uri.clone();
        metadata.version += 1;

        env.storage()
            .instance()
            .set(&DataKey::TokenMetadata(token_id), &metadata);

        // Add to history
        let history_entry = ConsentHistoryEntry {
            action: String::from_str(&env, "updated"),
            timestamp: env.ledger().timestamp(),
            actor: owner.clone(),
            metadata_uri: new_metadata_uri.clone(),
        };

        let mut history: Vec<ConsentHistoryEntry> = env
            .storage()
            .instance()
            .get(&DataKey::ConsentHistory(token_id))
            .unwrap_or(Vec::new(&env));
        history.push_back(history_entry);
        env.storage()
            .instance()
            .set(&DataKey::ConsentHistory(token_id), &history);

        // Emit event
        env.events().publish(
            (symbol_short!("consent"), symbol_short!("updated")),
            (token_id, metadata.version, new_metadata_uri),
        );
        Ok(())
    }

    /// Revoke consent (marks as revoked, prevents transfers)
    pub fn revoke_consent(env: Env, token_id: u64) {
        let owner: Address = env
            .storage()
            .instance()
            .get(&DataKey::TokenOwner(token_id))
            .expect("Token does not exist");

        // Verify caller is owner or authorized issuer
        owner.require_auth();

        // Mark as revoked
        env.storage()
            .instance()
            .set(&DataKey::TokenRevoked(token_id), &true);

        // Add to history
        let metadata: ConsentMetadata = env
            .storage()
            .instance()
            .get(&DataKey::TokenMetadata(token_id))
            .expect("Metadata not found");

        let history_entry = ConsentHistoryEntry {
            action: String::from_str(&env, "revoked"),
            timestamp: env.ledger().timestamp(),
            actor: owner.clone(),
            metadata_uri: metadata.metadata_uri.clone(),
        };

        let mut history: Vec<ConsentHistoryEntry> = env
            .storage()
            .instance()
            .get(&DataKey::ConsentHistory(token_id))
            .unwrap_or(Vec::new(&env));
        history.push_back(history_entry);
        env.storage()
            .instance()
            .set(&DataKey::ConsentHistory(token_id), &history);

        // Emit event
        env.events().publish(
            (symbol_short!("consent"), symbol_short!("revoked")),
            (token_id, owner),
        );
    }

    /// Transfer consent token (blocked if revoked)
    pub fn transfer(
        env: Env,
        from: Address,
        to: Address,
        token_id: u64,
    ) -> Result<(), ContractError> {
        from.require_auth();

        let owner: Address = env
            .storage()
            .instance()
            .get(&DataKey::TokenOwner(token_id))
            .expect("Token does not exist");

        if owner != from {
            return Err(ContractError::NotTokenOwner);
        }

        let is_revoked: bool = env
            .storage()
            .instance()
            .get(&DataKey::TokenRevoked(token_id))
            .unwrap_or(false);

        if is_revoked {
            return Err(ContractError::ConsentRevoked);
        }

        // Update ownership
        env.storage()
            .instance()
            .set(&DataKey::TokenOwner(token_id), &to);

        // Update token lists
        let from_key = DataKey::OwnerTokens(from.clone());
        let from_tokens: Vec<u64> = env
            .storage()
            .instance()
            .get(&from_key)
            .unwrap_or(Vec::new(&env));

        let mut new_from_tokens = Vec::new(&env);
        for i in 0..from_tokens.len() {
            let tid = from_tokens.get(i).unwrap();
            if tid != token_id {
                new_from_tokens.push_back(tid);
            }
        }
        env.storage().instance().set(&from_key, &new_from_tokens);

        let to_key = DataKey::OwnerTokens(to.clone());
        let mut to_tokens: Vec<u64> = env
            .storage()
            .instance()
            .get(&to_key)
            .unwrap_or(Vec::new(&env));
        to_tokens.push_back(token_id);
        env.storage().instance().set(&to_key, &to_tokens);

        // Emit event
        env.events().publish(
            (symbol_short!("consent"), symbol_short!("transfer")),
            (token_id, from, to),
        );
        Ok(())
    }

    /// Get token owner
    pub fn owner_of(env: Env, token_id: u64) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::TokenOwner(token_id))
            .expect("Token does not exist")
    }

    /// Get consent metadata
    pub fn get_metadata(env: Env, token_id: u64) -> ConsentMetadata {
        env.storage()
            .instance()
            .get(&DataKey::TokenMetadata(token_id))
            .expect("Token does not exist")
    }

    /// Check if consent is revoked
    pub fn is_revoked(env: Env, token_id: u64) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::TokenRevoked(token_id))
            .unwrap_or(false)
    }

    /// Get consent history (audit trail)
    pub fn get_history(env: Env, token_id: u64) -> Vec<ConsentHistoryEntry> {
        env.storage()
            .instance()
            .get(&DataKey::ConsentHistory(token_id))
            .unwrap_or(Vec::new(&env))
    }

    /// Get all tokens owned by an address
    pub fn tokens_of_owner(env: Env, owner: Address) -> Vec<u64> {
        env.storage()
            .instance()
            .get(&DataKey::OwnerTokens(owner))
            .unwrap_or(Vec::new(&env))
    }

    /// Check if consent is valid (not revoked and not expired)
    pub fn is_valid(env: Env, token_id: u64) -> bool {
        let is_revoked: bool = env
            .storage()
            .instance()
            .get(&DataKey::TokenRevoked(token_id))
            .unwrap_or(false);

        if is_revoked {
            return false;
        }

        let metadata: ConsentMetadata = env
            .storage()
            .instance()
            .get(&DataKey::TokenMetadata(token_id))
            .expect("Token does not exist");

        if metadata.expiry_timestamp == 0 {
            return true; // No expiry
        }

        env.ledger().timestamp() < metadata.expiry_timestamp
    }
}

#[cfg(test)]
mod test;
