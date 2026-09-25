//! JavaScript and TypeScript: declared, bound and registered functions, and JSX.
use super::*;

const VIEW: &str = "export interface Row { id: string }\nexport const label = (row: Row): string => row.id.trim();\nexport class View {\n  render(row: Row) { return <Cell value={label(row)} />; }\n}\nfunction plain() { return 1; }\n";

#[test]
fn callbacks_registered_through_calls_are_units_named_by_their_declaration() {
    let source = "export const myJourney = database.view({ public: true }, t.array(row), (ctx) => {\n  const account = activeAccount(ctx)\n  if (!account) return []\n  return [account]\n})\nconst save = useCallback(async () => {\n  await store.save()\n}, [store])\nconst Panel = memo(forwardRef((props, ref) => {\n  return render(props, ref)\n}))\nconst table = database.table({ name: 'x' })\n";
    let units = parse(Path::new("views.ts"), source).unwrap().units;
    let named: Vec<(&str, bool)> = units
        .iter()
        .map(|u| (u.name.as_str(), u.body.is_some()))
        .collect();
    assert_eq!(
        named,
        [("myJourney", true), ("save", true), ("Panel", true)]
    );
    assert!(units[0].calls.contains("activeAccount"));
}

#[test]
fn callbacks_registered_at_module_level_are_units_named_by_their_registration() {
    let source = "const app = new Hono()\napp.post('/pages', validator('form', (v, c) => check(v)), async (c) => {\n  const page = await insertPage(c.env.DB)\n  return c.json(page)\n})\nrouter.route('/items').get((req, res) => {\n  res.send(list())\n}).delete(async (req, res) => {\n  await remove(req.params.id)\n})\napp.use(async (c, next) => {\n  await next()\n})\ndescribe('pages', () => {\n  it('saves', () => {})\n})\ntest.beforeEach(async () => {})\nvi.mock('./db', () => ({ query: vi.fn() }))\nexport const getStaticPaths = (async ({ paginate }) => {\n  return paginate(await posts())\n}) satisfies GetStaticPaths\nexport default {\n  async fetch(request, env) {\n    return app.fetch(request, env)\n  },\n}\n";
    let units = parse(Path::new("routes.ts"), source).unwrap().units;
    let named: Vec<(&str, usize)> = units.iter().map(|u| (u.name.as_str(), u.line)).collect();
    assert_eq!(
        named,
        [
            ("app.post('/pages')", 2),
            ("router.get(…)", 6),
            ("router.delete(…)", 8),
            ("app.use(…)", 11),
            ("getStaticPaths", 19),
            ("fetch", 23),
        ]
    );
    assert!(units[0].calls.contains("insertPage"));
}

#[test]
fn typescript_units_include_interfaces_bound_arrows_and_methods() {
    assert_eq!(
        names("view.tsx", VIEW),
        [
            ("Row".into(), Kind::Type),
            ("label".into(), Kind::Function),
            ("View::render".into(), Kind::Method),
            ("plain".into(), Kind::Function),
        ]
    );
}

#[test]
fn jsx_components_count_as_calls() {
    let render = parse(Path::new("view.tsx"), VIEW).unwrap().units.remove(2);
    assert!(render.calls.contains("Cell") && render.calls.contains("label"));
}
