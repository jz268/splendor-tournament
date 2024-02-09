use crate::card::{Card, CardId};
use crate::color::Color;
use crate::nobles::*;
use crate::player::Player;
use crate::token::Tokens;

use rand::seq::SliceRandom;
use rand::thread_rng;

use self::Action::*;

use std::collections::HashSet;

use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Game {
    players: Vec<Player>,
    tokens: Tokens,
    decks: Vec<Vec<Card>>,
    current_player: usize,
    nobles: Vec<Noble>,
    dealt_cards: Vec<Vec<CardId>>,
    current_phase: Phase,
    card_lookup: Arc<Vec<Card>>,
}

// Note: I don't really know where to write this but the thought just occurred to me
// when doing MCTS, we have to be careful about how blind reserves appear to other players
// as that information has to be randomized every time.
// The reason being is that agents in a real game would not have perfect information
// about what was reserved, but can in fact make eliminations instead

impl Game {
    pub fn new(players: u8, card_lookup: Arc<Vec<Card>>) -> Game {
        let mut decks = Vec::new();
        for tier in 1..=3 {
            let mut deck = Vec::new();
            for card in Card::all() {
                if card.tier() == tier {
                    deck.push(card);
                }
            }
            decks.push(deck);
        }

        let mut nobles = Noble::all();
        nobles.shuffle(&mut thread_rng());
        nobles.truncate(players as usize + 1);

        let mut dealt_cards = Vec::<Vec<CardId>>::new();

        decks[0].shuffle(&mut thread_rng());
        decks[1].shuffle(&mut thread_rng());
        decks[2].shuffle(&mut thread_rng());

        // Deal 4 cards to start
        dealt_cards.push(decks[0].drain(0..4).map(|card| card.id()).collect());
        dealt_cards.push(decks[1].drain(0..4).map(|card| card.id()).collect());
        dealt_cards.push(decks[2].drain(0..4).map(|card| card.id()).collect());

        Game {
            players: (0..players).map(|_| Player::new()).collect(),
            tokens: Tokens::start(players),
            decks,
            current_player: 0,
            nobles,
            current_phase: Phase::PlayerStart,
            dealt_cards,
            card_lookup,
        }
    }

    fn is_phase_correct_for(&self, action: Action) -> bool {
        match self.current_phase {
            Phase::PlayerStart => match action {
                TakeDouble(_) => true,
                TakeDistinct(_) => true,
                Reserve(_) => true,
                ReserveHidden(_) => true,
                Purchase(_) => true,
                _ => false,
            },
            Phase::PlayerTokenCapExceeded => match action {
                Discard(_) => true,
                _ => false,
            },
            Phase::NobleAction => match action {
                AttractNoble(_) => true,
                _ => false,
            },
            Phase::PlayerActionEnd => match action {
                Continue => true,
                _ => false,
            },
        }
    }

    /// Deals a card to a certain tier and return the id
    /// Deals no card if the deck for that tier is exhausted
    fn deal_to(&mut self, tier: usize) -> Option<CardId> {
        if self.decks[tier].len() == 0 {
            return None;
        }
        let new_card = self.decks[tier].pop().unwrap();
        self.dealt_cards[tier].push(new_card.id());
        Some(new_card.id())
    }

    fn has_card(&self, card_id: CardId) -> bool {
        for tier in &self.dealt_cards {
            if tier.contains(&card_id) {
                return true;
            }
        }
        false
    }

    /// Removes a faceup card from the board
    /// and return the tier it was removed from
    fn remove_card(&mut self, card_id: CardId) -> usize {
        debug_assert!(self.has_card(card_id));

        let mut remove_index = (5, 5);
        for (tier, tiers) in self.dealt_cards.iter().enumerate() {
            for (index, id) in tiers.iter().enumerate() {
                if *id == card_id {
                    remove_index = (tier, index);
                }
            }
        }

        let (i, j) = remove_index;
        self.dealt_cards[i].remove(j);
        i
    }

    pub fn take_action(&mut self, action: Action) {
        debug_assert!(self.is_phase_correct_for(action.clone()));

        let next_phase = match action {
            TakeDouble(color) => {
                // Preconditions:
                // -> Must be from a pile that has >= 4
                // -> Cannot take a wild token with this action
                debug_assert!(self.tokens[color] >= 4);
                debug_assert!(!matches!(color, Color::Gold));

                // TODO: this is a little weird but we can change later
                // right now it's using debug asserts to check preconditions
                self.tokens -= Tokens::one(color);
                self.tokens -= Tokens::one(color);

                let player = &mut self.players[self.current_player];
                player.add_gems(Tokens::one(color));
                player.add_gems(Tokens::one(color));

                if player.gems().total() > 10 {
                    Phase::PlayerTokenCapExceeded
                } else {
                    Phase::NobleAction
                }
            }

            TakeDistinct(colors) => {
                // Preconditions
                // -> Can take 1,2, or 3 distinct colors
                debug_assert!(colors.len() <= 3 && colors.len() > 0);
                // -> Which all exist on the board
                debug_assert!(colors.iter().all(|c| self.tokens[*c] >= 1));
                // -> And you can only choose 2 or 1 tokens if all other
                // piles are depleted (See Splendor FAQ)
                debug_assert!(if colors.len() < 3 {
                    self.tokens.piles() == colors.len()
                } else {
                    true
                });
                // -> Cannot take a wild token with this action
                debug_assert!(colors.iter().all(|c| !matches!(c, Color::Gold)));

                let player = &mut self.players[self.current_player];
                player.add_gems(Tokens::from_set(&colors));

                for color in colors {
                    self.tokens -= Tokens::one(color);
                }

                if player.gems().total() > 10 {
                    Phase::PlayerTokenCapExceeded
                } else {
                    Phase::NobleAction
                }
            }

            Reserve(card_id) => {
                // Preconditions
                // -> Card with id:card_id is on the board
                debug_assert!(self.dealt_cards.iter().flatten().any(|id| card_id == *id));

                let tier = self.remove_card(card_id);
                self.deal_to(tier);

                // See if the player gets an wild/gold gem
                let gets_gold = self.tokens[Color::Gold] > 0;
                let player = &mut self.players[self.current_player];
                player.reserve_card(card_id);

                if gets_gold {
                    player.add_gems(Tokens::one(Color::Gold));
                    self.tokens -= Tokens::one(Color::Gold);
                }

                if player.gems().total() > 10 {
                    Phase::PlayerTokenCapExceeded
                } else {
                    Phase::NobleAction
                }
            }

            ReserveHidden(tier) => {
                let new_card_id = self.deal_to(tier).expect("Cannot reserve from empty deck");
                self.remove_card(new_card_id);

                let gets_gold = self.tokens[Color::Gold] > 0;
                let player = &mut self.players[self.current_player];

                if gets_gold {
                    player.add_gems(Tokens::one(Color::Gold));
                    self.tokens -= Tokens::one(Color::Gold);
                }

                player.blind_reserve_card(new_card_id);

                if player.gems().total() > 10 {
                    Phase::PlayerTokenCapExceeded
                } else {
                    Phase::NobleAction
                }
            }

            Purchase((card_id, payment)) => {
                let card = self.card_lookup[card_id as usize];
                let player = &self.players[self.current_player];
                // Preconditions:
                // -> The tokens being used is one of the legal ways to purchase this card
                debug_assert!({
                    let payment_options = player.payment_to_afford(&card);
                    let payment_options = payment_options.unwrap_or(HashSet::new());
                    payment_options.iter().any(|&option| option == payment)
                });
                // -> Must have been on the board or in the player's reserved cards
                debug_assert!(self.has_card(card_id) || player.has_reserved_card(card_id));

                let player = &mut self.players[self.current_player];
                player.purchase_card(&card, &payment);
                if self.has_card(card_id) {
                    let tier = self.remove_card(card_id);
                    self.deal_to(tier);
                }

                Phase::NobleAction
            }

            Discard(discards) => {
                // Preconditions:
                // -> Must have greater than 10 tokens 
                // -> Must discard enough tokens to be == 10
                // -> Must be discarding tokens already present in the player's gems
                let player = &mut self.players[self.current_player];
                debug_assert!(player.gems().total() > 10);
                debug_assert!(player.gems().total() as usize - discards.len() == 10);
                debug_assert!((*player.gems() - Tokens::from_vec(&discards)).legal());

                player.remove_gems(Tokens::from_vec(&discards));

                Phase::NobleAction
            }

            AttractNoble(noble_id) => { 

                // Preconditions:
                // -> The player has enough development cards to attract the noble
                let player = &self.players[self.current_player];
                let noble_index = self.nobles.iter().position(|n| n.id() == noble_id).unwrap();
                let noble = &self.nobles[noble_index];
                debug_assert!(noble.is_attracted_to(player.developments()));

                player.add_points(noble.points());
                self.nobles.remove(noble_index);

                Phase::PlayerActionEnd
            }

            _ => unimplemented!(),
        };
        self.current_phase = next_phase;
    }
}

#[derive(Debug, Clone)]
pub enum Phase {
    PlayerStart,            // Take some player action
    PlayerTokenCapExceeded, // [Optional] Player has > 10 tokens
    NobleAction,            // See if any nobles get attracted (multiple may be attracted)
    PlayerActionEnd,        // Finish the turn and see if the round should continue
}

#[derive(Debug, Clone)]
pub enum Action {
    TakeDouble(Color),
    TakeDistinct(HashSet<Color>),
    Reserve(CardId),
    ReserveHidden(usize),
    Purchase((CardId, Tokens)),

    Discard(Vec<Color>),

    AttractNoble(NobleId),

    /// Marker for passing the turn to the next player
    Continue,
}
