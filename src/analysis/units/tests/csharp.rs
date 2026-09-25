//! C#: classes, records and their members, and top-level statements.
use super::*;

#[test]
fn csharp_classes_records_and_their_members_are_units_with_facts() {
    let source = "using System.Data.SqlClient;\nusing Db = Microsoft.EntityFrameworkCore;\n\nnamespace Shop.Orders;\n\n/// <summary>Stores orders.</summary>\n[Route(\"api/[controller]\")]\npublic class OrderStore : Controller\n{\n    private const int MaxRows = 500;\n    private static readonly string Host = \"https://api.example.com\";\n    private readonly IOrderRepository _repository;\n\n    public OrderStore(IOrderRepository repository)\n    {\n        _repository = repository;\n    }\n\n    public int Count { get { return _repository.Count(); } }\n    public string Name { get; set; }\n\n    [HttpGet(\"search\")]\n    public async Task<IActionResult> Search(string keyword)\n    {\n        var query = $\"SELECT * FROM Products WHERE name LIKE '%{keyword}%'\";\n        using var command = new SqlCommand(query, _connection);\n        if (keyword == null)\n        {\n            throw new ArgumentException(\"empty keyword\");\n        }\n        else if (keyword == \"root\")\n        {\n            return BadRequest();\n        }\n        foreach (var item in await _repository.ListAsync<Order>())\n        {\n            switch (item.Kind) { case 3: break; }\n        }\n        return Ok(string.Format(\"{0} rows\", MaxRows));\n    }\n}\n\npublic record Person(string First, string Last);\n\npublic interface IOrderRepository { int Count(); }\n";
    let file = parse(Path::new("OrderStore.cs"), source).unwrap();
    let named: Vec<(&str, Kind, usize)> = file
        .units
        .iter()
        .map(|u| (u.name.as_str(), u.kind, u.line))
        .collect();
    assert_eq!(
        named,
        [
            ("OrderStore::OrderStore", Kind::Method, 14),
            ("OrderStore::Count", Kind::Method, 19),
            ("OrderStore::Search", Kind::Method, 22),
            ("Person", Kind::Type, 43),
            ("IOrderRepository", Kind::Type, 45),
        ]
    );
    assert_imports(&file, &["SqlClient", "Db"]);
    let search = &file.units[2];
    assert_eq!(
        search.signature,
        "public async Task<IActionResult> Search(string keyword)"
    );
    assert!(search.calls.contains("ListAsync") && search.calls.contains("SqlCommand"));
    assert!(search.refs.contains("IActionResult") && search.refs.contains("Order"));
    assert!(
        search.sites[0].text.starts_with("var query = $\"SELECT"),
        "{:?}",
        search.sites
    );
    assert_flow(
        search,
        (2, 2),
        &[("ArgumentException", "\"empty keyword\"")],
    );
    assert!(search.literals.iter().any(|l| l.text == "\"root\""));
    assert!(!search.literals.iter().any(|l| l.text.contains("search")));
    assert_eq!(
        constant_names(&file),
        ["OrderStore.MaxRows", "OrderStore.Host"]
    );
    let constructor = &file.units[0];
    assert!(constructor.too_small());
    assert_eq!(constructor.doc, "");
    let documented = parse(Path::new("A.cs"), "public class A\n{\n    /// <summary>\n    /// Loads orders.\n    /// </summary>\n    void Run() { }\n}\n").unwrap();
    assert_eq!(documented.units[0].doc, "Loads orders.");
    assert!(crate::syntax::supported(Path::new("OrderStore.cs")));
}

#[test]
fn csharp_top_level_statements_register_route_handlers_and_keep_setup() {
    let source = "var builder = WebApplication.CreateBuilder(args);\nbuilder.Services.AddCors(o => o.AddPolicy(\"all\", p => p.AllowAnyOrigin()));\nvar app = builder.Build();\napp.MapGet(\"/orders/{id}\", async (int id, OrderDb db) =>\n{\n    var order = await db.Orders.FindAsync(id);\n    return order is null ? Results.NotFound() : Results.Ok(order);\n}).RequireAuthorization();\napp.Use(async (context, next) => { await next(context); });\nstatic string Greet(string name)\n{\n    return $\"Hello {name}\";\n}\napp.Run();\n";
    let file = parse(Path::new("Program.cs"), source).unwrap();
    let named: Vec<(&str, usize)> = file
        .units
        .iter()
        .map(|u| (u.name.as_str(), u.line))
        .collect();
    assert_eq!(
        named,
        [
            ("app.MapGet(\"/orders/{id}\")", 4),
            ("app.Use(…)", 9),
            ("Greet", 10)
        ]
    );
    assert!(file.units[0].calls.contains("FindAsync"));
    let setup: Vec<usize> = file.setup.statements.iter().map(|s| s.1).collect();
    assert_eq!(setup, [1, 2, 3, 14]);
    assert!(
        file.setup.sites[1]
            .text
            .starts_with("builder.Services.AddCors(")
    );
}
