#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {

	use frame_support::{
		pallet_prelude::{
			ensure, BoundedVec, Decode, DispatchError, DispatchResult, Encode, Get, IsType,
			MaxEncodedLen, RuntimeDebug, StorageMap, StorageValue, Twox64Concat, TypeInfo,
			ValueQuery,
		},
		traits::{Currency, Randomness},
	};

	use frame_system::pallet_prelude::*;

	pub(crate) type BalanceOf<T> =
		<<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

	#[derive(Clone, Encode, Decode, PartialEq, Copy, RuntimeDebug, TypeInfo, MaxEncodedLen)]
	pub enum Color {
		Red,
		Yellow,
		Blue,
		Green,
	}

	#[derive(Clone, Encode, Decode, PartialEq, Copy, RuntimeDebug, TypeInfo, MaxEncodedLen)]
	#[scale_info(skip_type_params(T))]
	pub struct Collectible<T: Config> {
		// Unsigned integers of 16 bytes to represent a unique identifier
		pub unique_id: [u8; 16],
		// `None` assumes not for sale
		pub price: Option<BalanceOf<T>>,
		pub color: Color,
		pub owner: T::AccountId,
	}

	// Pallet error messages.
	#[pallet::error]
	pub enum Error<T> {
		/// Each collectible must have a unique identifier
		DuplicateCollectible,
		/// An account can't exceed the `MaximumOwned` constant
		MaximumCollectiblesOwned,
		/// The total supply of collectibles can't exceed the u64 limit
		BoundsOverflow,
		/// The collectible doesn't exist
		NoCollectible,
		/// You are not the owner
		NotOwner,
		/// Trying to transfer a collectible to yourself
		TransferToSelf,
		/// The bid is lower than the asking price.
		BidPriceTooLow,
		/// The collectible is not for sale.
		NotForSale,
		/// Not enough fund to buy
		NotEnoughFund,
	}

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	#[pallet::storage]
	#[pallet::getter(fn collectibles_count)]
	pub(super) type CollectiblesCount<T: Config> = StorageValue<_, u64, ValueQuery>;

	/// Maps the Collectible struct to the unique_id.
	#[pallet::storage]
	#[pallet::getter(fn collectible_map)]
	pub(super) type CollectibleMap<T: Config> =
		StorageMap<_, Twox64Concat, [u8; 16], Collectible<T>>;

	/// Track the collectibles owned by each account.
	#[pallet::storage]
	#[pallet::getter(fn owner_of_collectibles)]
	pub(super) type OwnerOfCollectibles<T: Config> = StorageMap<
		_,
		Twox64Concat,
		T::AccountId,
		BoundedVec<[u8; 16], T::MaximumOwned>,
		ValueQuery,
	>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// A new collectible was successfully created.
		CollectibleCreated { collectible: [u8; 16], owner: T::AccountId },
		/// A collectible was successfully transferred.
		TransferSucceeded { from: T::AccountId, to: T::AccountId },
		/// The price of a collectible was successfully set.
		PriceSet { collectible: [u8; 16], price: Option<BalanceOf<T>> },
		/// A collectible was successfully sold.
		Sold {
			seller: T::AccountId,
			buyer: T::AccountId,
			collectible: [u8; 16],
			price: BalanceOf<T>,
		},
	}

	#[pallet::config]
	pub trait Config: frame_system::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		type CollectionRandomness: Randomness<Self::Hash, Self::BlockNumber>;
		type Currency: Currency<Self::AccountId>;
		// type Currency: ReservableCurrency<Self::AccountId>
		// 	+ LockableCurrency<Self::AccountId, Moment = Self::BlockNumber>;

		#[pallet::constant]
		type MaximumOwned: Get<u32>;
	}

	// Pallet internal functions
	impl<T: Config> Pallet<T> {
		// Generates and returns the unique_id and color
		fn gen_unique_id() -> ([u8; 16], Color) {
			// Create randomness
			let random = T::CollectionRandomness::random(&b"unique_id"[..]).0;

			// Create randomness payload. Multiple collectibles can be generated in the same block,
			// retaining uniqueness.
			let unique_payload = (
				random,
				frame_system::Pallet::<T>::extrinsic_index().unwrap_or_default(),
				frame_system::Pallet::<T>::block_number(),
			);

			// Turns into a byte array
			let encoded_payload = unique_payload.encode();
			let hash = frame_support::Hashable::blake2_128(&encoded_payload);

			// Generate Color
			if hash[0] % 2 == 0 {
				(hash, Color::Red)
			} else {
				(hash, Color::Yellow)
			}
		}
		// Function to mint a collectible
		pub fn mint(
			owner: &T::AccountId,
			unique_id: [u8; 16],
			color: Color,
		) -> Result<[u8; 16], DispatchError> {
			// Create a new object
			let collectible =
				Collectible::<T> { unique_id, price: None, color, owner: owner.clone() };

			// Check if the collectible exists in the storage map
			ensure!(
				!CollectibleMap::<T>::contains_key(&collectible.unique_id),
				Error::<T>::DuplicateCollectible
			);

			// Check that a new collectible can be created
			let count = CollectiblesCount::<T>::get();
			let new_count = count.checked_add(1).ok_or(Error::<T>::BoundsOverflow)?;

			// Append collectible to OwnerOfCollectibles map
			OwnerOfCollectibles::<T>::try_append(&owner, collectible.unique_id)
				.map_err(|_| Error::<T>::MaximumCollectiblesOwned)?;

			// Write new collectible to storage and update the count
			CollectibleMap::<T>::insert(collectible.unique_id, collectible);
			CollectiblesCount::<T>::put(new_count);

			// Deposit the "Collectiblereated" event.
			Self::deposit_event(Event::CollectibleCreated {
				collectible: unique_id,
				owner: owner.clone(),
			});

			// Returns the unique_id of the new collectible if this succeeds
			Ok(unique_id)
		}

		// Update storage to transfer collectible
		pub fn do_transfer(collectible_id: [u8; 16], to: T::AccountId) -> DispatchResult {
			// Get the collectible
			let mut collectible =
				CollectibleMap::<T>::get(&collectible_id).ok_or(Error::<T>::NoCollectible)?;
			let from = collectible.owner;

			ensure!(from != to, Error::<T>::TransferToSelf);
			let mut from_owned = OwnerOfCollectibles::<T>::get(&from);

			// Remove collectible from list of owned collectible.
			if let Some(ind) = from_owned.iter().position(|&id| id == collectible_id) {
				from_owned.swap_remove(ind);
			} else {
				return Err(Error::<T>::NoCollectible.into());
			}

			// Add collectible to the list of owned collectibles.
			let mut to_owned = OwnerOfCollectibles::<T>::get(&to);
			to_owned
				.try_push(collectible_id)
				.map_err(|_collectible_id| Error::<T>::MaximumCollectiblesOwned)?;

			// Transfer succeeded, update the owner and reset the price to `None`.
			collectible.owner = to.clone();
			collectible.price = None;

			// Write updates to storage
			CollectibleMap::<T>::insert(&collectible_id, collectible);
			OwnerOfCollectibles::<T>::insert(&to, to_owned);
			OwnerOfCollectibles::<T>::insert(&from, from_owned);

			Self::deposit_event(Event::TransferSucceeded { from, to });
			Ok(())
		}

		fn do_buy_collectible(
			to: T::AccountId,
			collectible_id: [u8; 16],
			bid_price: Option<BalanceOf<T>>,
		) -> DispatchResult {
			// Get the collectible
			let mut collectible =
				CollectibleMap::<T>::get(&collectible_id).ok_or(Error::<T>::NoCollectible)?;
			let from = collectible.owner;

			ensure!(from != to, Error::<T>::TransferToSelf);
			ensure!(collectible.price != None, Error::<T>::NotForSale);
			ensure!(bid_price >= collectible.price, Error::<T>::BidPriceTooLow);

			let mut from_owned = OwnerOfCollectibles::<T>::get(&from);

			// Remove collectible from list of owned collectible.
			if let Some(ind) = from_owned.iter().position(|&id| id == collectible_id) {
				from_owned.swap_remove(ind);
			} else {
				return Err(Error::<T>::NoCollectible.into());
			}

			// set new owner for collection
			// Add collectible to the list of owned collectibles.
			let mut to_owned = OwnerOfCollectibles::<T>::get(&to);
			to_owned
				.try_push(collectible_id)
				.map_err(|_collectible_id| Error::<T>::MaximumCollectiblesOwned)?;

			//make payment
			if let Some(price) = collectible.price {
				// let to_balance = T::Currency::total_issuance();
				// ensure!(Some(to_balance) >= bid_price, Error::<T>::NotEnoughFund);

				T::Currency::transfer(
					&to,
					&from,
					price,
					frame_support::traits::ExistenceRequirement::KeepAlive,
				)?;
				// Deposit sold event
				Self::deposit_event(Event::Sold {
					seller: from.clone(),
					buyer: to.clone(),
					collectible: collectible_id,
					price,
				});
			};

			// Transfer succeeded, update the owner and reset the price to `None`.
			collectible.owner = to.clone();
			collectible.price = None;

			// Write updates to storage
			CollectibleMap::<T>::insert(&collectible_id, collectible);
			OwnerOfCollectibles::<T>::insert(&to, to_owned);
			OwnerOfCollectibles::<T>::insert(&from, from_owned);
			Self::deposit_event(Event::TransferSucceeded { from, to });

			Ok(())
		}
	}

	// Pallet callable functions
	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Create a new unique collectible.
		///
		/// The actual collectible creation is done in the `mint()` function.
		#[pallet::weight(0)]
		pub fn create_collectible(origin: OriginFor<T>) -> DispatchResult {
			// Make sure the caller is from a signed origin
			let sender = ensure_signed(origin)?;

			// Generate the unique_id and color using a helper function
			let (collectible_gen_unique_id, color) = Self::gen_unique_id();

			// Write new collectible to storage by calling helper function
			Self::mint(&sender, collectible_gen_unique_id, color)?;

			Ok(())
		}

		/// Transfer a collectible to another account.
		/// Any account that holds a collectible can send it to another account.
		/// Transfer resets the price of the collectible, marking it not for sale.
		#[pallet::weight(0)]
		pub fn transfer(
			origin: OriginFor<T>,
			to: T::AccountId,
			unique_id: [u8; 16],
		) -> DispatchResult {
			// Make sure the caller is from a signed origin
			let from = ensure_signed(origin)?;
			let collectible =
				CollectibleMap::<T>::get(&unique_id).ok_or(Error::<T>::NoCollectible)?;
			ensure!(collectible.owner == from, Error::<T>::NotOwner);
			Self::do_transfer(unique_id, to)?;
			Ok(())
		}

		/// Update the collectible price and write to storage.
		#[pallet::weight(0)]
		pub fn set_price(
			origin: OriginFor<T>,
			collectible_id: [u8; 16],
			new_price: Option<BalanceOf<T>>,
		) -> DispatchResult {
			let from = ensure_signed(origin)?;
			// Get the collectible
			let mut collectible =
				CollectibleMap::<T>::get(&collectible_id).ok_or(Error::<T>::NoCollectible)?;

			// Get owner
			let _owner = collectible.owner.clone();
			// Guarantee owner is caller
			ensure!(from == _owner, Error::<T>::NotOwner);

			// set price for collectible
			collectible.price = new_price;
			CollectibleMap::<T>::insert(collectible.unique_id, collectible);

			// Deposit event
			Self::deposit_event(Event::PriceSet { collectible: collectible_id, price: new_price });

			Ok(())
		}

		/// Buy a collectible. The bid price must be greater than or equal to the price
		/// set by the collectible owner.
		#[pallet::weight(0)]
		pub fn buy_collectible(
			origin: OriginFor<T>,
			unique_id: [u8; 16],
			bid_price: BalanceOf<T>,
		) -> DispatchResult {
			// Make sure the caller is from a signed origin
			let buyer = ensure_signed(origin)?;
			// Transfer the collectible from seller to buyer.
			Self::do_buy_collectible(buyer, unique_id, Some(bid_price))?;
			Ok(())
		}
	}
}
