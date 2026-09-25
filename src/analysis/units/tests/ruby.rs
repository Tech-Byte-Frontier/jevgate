//! Ruby: methods of their class or module, and blocks named by their call.
use super::*;

const BILLING: &str = "require 'json'\nrequire_relative 'billing/tax'\n\nmodule Billing\n  RETRIES = 3\n  ROUND = ->(value) { value.round(2) }\n\n  # Builds invoices.\n  class Invoice < Base\n    TAX = 0.21\n\n    def self.build(rows)\n      new(rows).tap(&:validate)\n    end\n\n    class << self\n      def empty\n        new([])\n      end\n    end\n\n    def total(discount = 0)\n      raise ArgumentError, \"negative discount\" if discount.negative?\n      sum = rows.sum { |row| row.price }\n      if sum > 1000\n        sum * (1 - TAX)\n      elsif sum > 100\n        sum\n      else\n        raise Billing::Error.new(\"too small\")\n      end\n    end\n\n    private def rows\n      @rows\n    end\n\n    define_method(:currency) do\n      Currency.new(\"EUR\").code\n    end\n  end\nend\n\nget '/invoices' do\n  Billing::Invoice.build(params).to_json\nend\n\nRSpec.configure do |config|\n  config.order = :random\nend\n";

#[test]
fn ruby_methods_follow_their_class_or_module_and_blocks_are_named_by_their_call() {
    let file = parse(Path::new("billing.rb"), BILLING).unwrap();
    let named: Vec<(&str, &str, usize)> = file
        .units
        .iter()
        .map(|u| (u.name.as_str(), u.owner.as_str(), u.line))
        .collect();
    assert_eq!(
        named,
        [
            ("Billing::ROUND", "Billing", 6),
            ("Invoice::build", "Invoice", 12),
            ("Invoice::empty", "Invoice", 17),
            ("Invoice::total", "Invoice", 22),
            ("Invoice::rows", "Invoice", 34),
            ("Invoice::currency", "Invoice", 38),
            ("get('/invoices')", "", 44),
        ],
        "`RSpec.configure` sets up tests and is not a unit"
    );
    assert_imports(&file, &["json", "tax"]);
    let total = &file.units[3];
    assert_eq!(total.signature, "def total(discount = 0)");
    assert_flow(
        total,
        (1, 3),
        &[
            ("ArgumentError", "\"negative discount\""),
            ("Billing::Error", "\"too small\""),
        ],
    );
    assert!(total.calls.contains("sum") && total.refs.contains("TAX"));
    assert!(
        file.units[5].calls.contains("Currency"),
        "`Currency.new` builds a Currency"
    );
    assert_eq!(constant_names(&file), ["RETRIES", "TAX"]);
    assert!(
        file.units
            .iter()
            .all(|u| u.literals.iter().all(|l| !l.text.contains("tax"))),
        "required paths are not values"
    );
}

#[test]
fn ruby_test_blocks_are_left_to_the_test_rules_but_their_helpers_are_units() {
    let source = "describe Invoice do\n  let(:invoice) { Invoice.new }\n\n  def build_rows(count)\n    Array.new(count) { Row.new }\n  end\n\n  it \"totals\" do\n    expect(invoice.total).to eq 0\n  end\nend\n";
    let names: Vec<String> = parse(Path::new("invoice_spec.rb"), source)
        .unwrap()
        .units
        .into_iter()
        .map(|u| u.name)
        .collect();
    assert_eq!(names, ["build_rows"]);
}

#[test]
fn a_block_on_a_long_receiver_is_named_with_its_arguments_shortened() {
    let source = "Comment.where(\n  \"id > ?\", last_id\n).order(:id).find_each do |comment|\n  notify(comment)\n  comment.touch\nend\n";
    let file = parse(Path::new("script/mail.rb"), source).unwrap();
    assert_eq!(file.units[0].name, "Comment.where(…).order(…).find_each");
}
