//! What requests cost: a check's dollar budget, priced before each request
//! is sent, and what the provider billed for the answers.
use crate::{options::CheckArgs, response};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::collections::BTreeMap;

/// A dollar budget, `--max-cost`: each request is priced from its size
/// before it is first sent, at the model's price (an alias's at Jev 1.13's,
/// which the gateways charge too), and none is sent that would pass it.
pub struct Spend {
    budget: f64,
    spent: std::sync::Mutex<Spent>,
}

/// What the requests a check sent are estimated to cost.
#[derive(Default)]
struct Spent {
    usd: f64,
    /// The requests priced, by address: a retry sends the same request, and
    /// is not priced again.
    priced: std::collections::BTreeSet<usize>,
    /// The share of the budget last said to be spent, in percent.
    noticed: u8,
}

/// Shares of the budget stderr says are spent, in percent, highest first.
const SPEND_NOTICES: [u8; 2] = [90, 75];

impl Spend {
    pub fn new(budget: f64) -> Self {
        Self {
            budget,
            spent: Default::default(),
        }
    }

    /// Price `request` at `usd` the first time it is sent, or refuse it when
    /// that would pass the budget; say on stderr when the spend first
    /// passes 75% and 90% of it.
    pub(super) fn charge(
        &self,
        args: &CheckArgs,
        request: &Value,
        usd: impl FnOnce() -> f64,
    ) -> Result<()> {
        let mut spent = self.spent.lock().unwrap();
        let address = std::ptr::from_ref(request) as usize;
        if spent.priced.contains(&address) {
            return Ok(());
        }
        let cost = usd();
        ensure!(
            spent.usd + cost <= self.budget,
            "{} request not sent: it would pass the ${:.2} budget (max_cost)",
            args.provider.service().label,
            self.budget
        );
        spent.usd += cost;
        spent.priced.insert(address);
        let share = spent.usd / self.budget * 100.0;
        if let Some(notice) = SPEND_NOTICES
            .into_iter()
            .find(|&notice| share >= f64::from(notice) && spent.noticed < notice)
        {
            spent.noticed = notice;
            note!(
                "jevgate: {notice}% of the ${:.2} budget spent (about ${:.4})",
                self.budget,
                spent.usd
            );
        }
        Ok(())
    }
}

/// What one answered request was billed: the model that answered, and the
/// tokens its response reported.
pub(super) struct Billed {
    model: String,
    /// None when the response reported no usage.
    pub(super) input_tokens: Option<u64>,
    pub(super) output_tokens: u64,
}

impl Billed {
    /// What the provider's `body` says was billed. A model name that fails
    /// validation is billed to "unknown", which has no price.
    pub(super) fn of(body: &Value) -> Self {
        let model = body["model"]
            .as_str()
            .filter(|name| crate::model::valid_name(name))
            .unwrap_or("unknown");
        Self {
            model: model.to_owned(),
            input_tokens: response::input_tokens(body),
            output_tokens: response::output_tokens(body),
        }
    }
}

/// What this invocation's requests were billed, and by which models.
#[derive(Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Input tokens by the model that answered them.
    pub models: BTreeMap<String, u64>,
    /// Answers whose response reported no usage.
    pub unmetered: u32,
}

impl Usage {
    pub(super) fn bill(&mut self, billed: Billed) {
        self.output_tokens += billed.output_tokens;
        match billed.input_tokens {
            Some(tokens) => {
                self.input_tokens += tokens;
                *self.models.entry(billed.model).or_default() += tokens;
            }
            None => self.unmetered += 1,
        }
    }

    /// Dollars, priced by the model that answered each request; unknown when
    /// an answer reported no usage or a model has no published price. The
    /// fold starts at 0.0: a float `sum` of nothing is -0.0, shown as "$-0.0000".
    pub fn usd(&self) -> Option<f64> {
        if self.unmetered > 0 {
            return None;
        }
        self.models.iter().try_fold(0.0, |total, (model, tokens)| {
            Some(total + crate::model::usd(model, *tokens)?)
        })
    }
}
