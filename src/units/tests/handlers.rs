//! Web framework error handlers: how each is found and registered, and the
//! one question it is asked with the program's error classes.
use super::*;

#[test]
fn laravel_and_slim_error_handlers_are_found_by_the_class_they_extend() {
    let project = Project::new();
    project.write(
        "src/Handlers/HttpErrorHandler.php",
        "<?php\nnamespace App\\Handlers;\n\nuse Slim\\Handlers\\ErrorHandler as SlimErrorHandler;\n\nclass HttpErrorHandler extends SlimErrorHandler\n{\n    protected function respond(): Response\n    {\n        $response = $this->responseFactory->createResponse(500);\n        $response->getBody()->write($this->exception->getMessage());\n        return $response;\n    }\n}\n",
    );
    project.write(
        "app/Exceptions/Handler.php",
        "<?php\nnamespace App\\Exceptions;\n\nclass Handler extends ExceptionHandler\n{\n    public function render($request, Throwable $e)\n    {\n        return response()->json(['error' => $e->getMessage()], 500);\n    }\n}\n",
    );
    project.write(
        "public/index.php",
        "<?php\nset_exception_handler(function (Throwable $e) {\n    echo $e->getMessage();\n});\nclass Page extends Base { public function render() { return view('page'); } }\n",
    );
    // Only PHP classes and PHP registrations are read this way.
    project.write(
        "src/view.ts",
        "class Panel extends BaseErrorHandler {\n  render(error: Error) {\n    return `<p>${error.message}</p>`;\n  }\n}\nset_exception_handler((e: Error) => send(e.stack));\n",
    );
    let mut options = args();
    options.rules = vec![catalog::SENSITIVE_DATA.into()];
    let (_, plan) = planned(&project, &options);
    assert_eq!(
        registered_handlers(&plan),
        [
            "`class Handler extends ExceptionHandler` (app/Exceptions/Handler.php:6)",
            "`class HttpErrorHandler extends SlimErrorHandler` (src/Handlers/HttpErrorHandler.php:8)",
            "`set_exception_handler(function (Throwable $e) {\n    echo $e->getMessage();\n})` (public/index.php:2)",
        ]
    );
}

#[test]
fn a_registered_error_handler_is_one_unit_judged_with_the_error_classes() {
    let project = Project::new();
    project.write(
        "src/app.ts",
        "import { errorHandler } from './middleware/error-handler'\nconst app = new Hono()\napp.onError(errorHandler)\nexport default app\n",
    );
    project.write(
        "src/middleware/error-handler.ts",
        "export const errorHandler = (err, c) => {\n  logger.error(err)\n  if (err instanceof AppError) {\n    return c.json({ error: { code: err.code, message: err.message } }, err.status)\n  }\n  return c.json({ error: { message: err.message, stack: err.stack } }, 500)\n}\n",
    );
    project.write(
        "src/lib/errors.ts",
        "export class AppError extends Error {\n  constructor(public status: number, public code: string, message: string) {\n    super(message)\n  }\n}\n",
    );
    project.write(
        "src/app.test.ts",
        "import { errorHandler } from './middleware/error-handler'\ntest('x', () => {\n  app.onError(errorHandler)\n})\n",
    );
    let mut options = args();
    options.rules = vec![catalog::SENSITIVE_DATA.into()];
    let (inputs, plan) = planned(&project, &options);
    let handlers: Vec<(&std::path::Path, &UnitPlan)> = plan
        .files
        .values()
        .flat_map(|f| f.units.iter().map(move |u| (f.path.as_path(), u)))
        .filter(|(_, u)| matches!(u.detail, Detail::Handler { .. }))
        .collect();
    assert_eq!(handlers.len(), 1, "registered once outside tests");
    let (path, unit) = handlers[0];
    assert_eq!(
        path,
        std::path::Path::new("src/middleware/error-handler.ts")
    );
    assert_eq!(unit.name, "errorHandler");
    let request = &plan
        .requests
        .iter()
        .find(|p| p.request["state"]["error_handler"].is_object())
        .unwrap()
        .request;
    assert_eq!(
        request["state"]["error_handler"]["registered"],
        "`app.onError(errorHandler)` (src/app.ts:3)"
    );
    assert!(
        request["state"]["error_classes"]
            .as_str()
            .unwrap()
            .starts_with("export class AppError")
    );
    assert_eq!(request["jevgate"]["sources"].as_array().unwrap().len(), 2);
    assert!(inputs.len() >= 3);
    let report = run_with_nouls(&project, &options, &[("handler_leaks", 0.95)]);
    let file = report
        .files
        .iter()
        .find(|f| f.path.ends_with("error-handler.ts"))
        .unwrap();
    let finding = file
        .findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some("errorHandler") && f.strength == Strength::Review)
        .unwrap();
    assert!(
        finding
            .message
            .starts_with("`errorHandler`, the error handler registered by `app.onError(errorHandler)` (src/app.ts:3), sends clients"),
        "{}",
        finding.message
    );
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-209 error details exposed")
    );
}

#[test]
fn registrations_named_in_comments_or_strings_register_nothing() {
    let project = Project::new();
    project.write(
        "src/patterns.ts",
        "// Handlers are found where the program calls `.onError(handler)`.\nexport const REGISTRATIONS = ['.onError(', '.setErrorHandler(']\nexport function describe(app) {\n  return `app.onError(report)` + app.name\n}\n",
    );
    let mut options = args();
    options.rules = vec![catalog::SENSITIVE_DATA.into()];
    let (_, plan) = planned(&project, &options);
    assert!(
        !plan
            .files
            .values()
            .flat_map(|f| &f.units)
            .any(|u| matches!(u.detail, Detail::Handler { .. }))
    );
}

#[test]
fn framework_error_handlers_are_found_where_they_are_implemented_or_used() {
    let project = Project::new();
    project.write(
        "src/error.rs",
        "use axum::response::{IntoResponse, Response};\n\n#[derive(thiserror::Error, Debug)]\npub enum Error {\n    #[error(\"request path not found\")]\n    NotFound,\n    #[error(\"an internal server error occurred\")]\n    Anyhow(#[from] anyhow::Error),\n}\n\n#[derive(Debug)]\npub struct TimeoutError;\n\nimpl IntoResponse for Error {\n    fn into_response(self) -> Response {\n        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response()\n    }\n}\n\nimpl IntoResponse for Page {\n    fn into_response(self) -> Response {\n        Html(self.0).into_response()\n    }\n}\n",
    );
    project.write(
        "src/server.ts",
        "const app = express()\napp.use(express.json())\napp.use(cors({ origin: true }))\napp.use((err: Error, req: Request<{}, any>, res: Response, next: NextFunction) => {\n  res.status(500).json({ message: err.message })\n})\n",
    );
    project.write(
        "src/filter.ts",
        "@Catch(HttpException)\nexport class HttpErrorFilter implements ExceptionFilter {\n  catch(exception: HttpException, host: ArgumentsHost) {\n    host.switchToHttp().getResponse().status(500).json(exception.getResponse())\n  }\n}\n",
    );
    let mut options = args();
    options.rules = vec![catalog::SENSITIVE_DATA.into()];
    let (_, plan) = planned(&project, &options);
    let registered = registered_handlers(&plan);
    assert_eq!(registered.len(), 3, "{registered:?}");
    assert!(
        registered.contains(&"`impl IntoResponse for Error` (src/error.rs:15)".to_string()),
        "{registered:?}"
    );
    assert!(
        registered
            .iter()
            .any(|r| r.starts_with("`app.use((err: Error"))
    );
    assert!(
        registered.contains(&"`@Catch(…) class HttpErrorFilter` (src/filter.ts:3)".to_string())
    );
    let classes = plan
        .requests
        .iter()
        .find(|p| {
            p.request["state"]["error_handler"]["registered"]
                .as_str()
                .is_some_and(|r| r.contains("IntoResponse"))
        })
        .unwrap()
        .request["state"]["error_classes"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        classes.starts_with("#[derive(thiserror::Error, Debug)]\npub enum Error {"),
        "{classes}"
    );
    assert!(classes.contains("an internal server error occurred"));
    assert!(classes.ends_with("pub struct TimeoutError;"), "{classes}");
}

#[test]
fn aspnet_core_exception_handlers_are_found_where_they_are_implemented_or_registered() {
    let project = Project::new();
    project.write(
        "src/Api/Program.cs",
        "var app = WebApplication.CreateBuilder(args).Build();\napp.UseExceptionHandler(\"/Error\");\napp.UseExceptionHandler(errorApp =>\n{\n    errorApp.Run(async context =>\n    {\n        var error = context.Features.Get<IExceptionHandlerFeature>();\n        await context.Response.WriteAsync(error.Error.ToString());\n    });\n});\napp.UseMiddleware<ExceptionMiddleware>();\napp.Run();\n",
    );
    project.write(
        "src/Api/ExceptionMiddleware.cs",
        "namespace Api;\n\npublic class ExceptionMiddleware\n{\n    private readonly RequestDelegate _next;\n\n    public ExceptionMiddleware(RequestDelegate next) => _next = next;\n\n    public async Task InvokeAsync(HttpContext httpContext)\n    {\n        try\n        {\n            await _next(httpContext);\n        }\n        catch (Exception ex)\n        {\n            await Write(httpContext, ex);\n        }\n    }\n\n    private static Task Write(HttpContext context, Exception exception)\n    {\n        context.Response.StatusCode = 500;\n        return context.Response.WriteAsync(exception.Message);\n    }\n}\n",
    );
    project.write(
        "src/Api/Filters.cs",
        "namespace Api;\n\npublic class ApiExceptionFilter : IExceptionFilter\n{\n    public void OnException(ExceptionContext context)\n    {\n        context.Result = new ObjectResult(new ProblemDetails { Detail = context.Exception.StackTrace });\n        context.ExceptionHandled = true;\n    }\n}\n\npublic sealed class GlobalHandler(ILogger<GlobalHandler> logger) : IExceptionHandler\n{\n    public async ValueTask<bool> TryHandleAsync(HttpContext context, Exception exception, CancellationToken token)\n    {\n        logger.LogError(exception, \"Unhandled\");\n        await context.Response.WriteAsJsonAsync(new { title = \"Server error\" }, token);\n        return true;\n    }\n}\n\npublic class DuplicateException : Exception\n{\n    public DuplicateException(string message) : base(message) { }\n}\n\npublic class NotFoundException(string name) : Exception($\"{name} was not found\");\n",
    );
    let mut options = args();
    options.rules = vec![catalog::SENSITIVE_DATA.into()];
    let (_, plan) = planned(&project, &options);
    assert_eq!(
        registered_handlers(&plan),
        [
            "`app.UseExceptionHandler(errorApp =>\n{\n    errorApp.Run(async context =>\n    {\n        var error = context.Features.Get<IExceptionHandlerFeature>();\n        await context.Response.WriteAsync(error.Error.ToString());\n    });\n})` (src/Api/Program.cs:3)",
            "`class ApiExceptionFilter : IExceptionFilter` (src/Api/Filters.cs:5)",
            "`class GlobalHandler : IExceptionHandler` (src/Api/Filters.cs:14)",
            "`middleware class ExceptionMiddleware` (src/Api/ExceptionMiddleware.cs:9)",
        ],
        "a path passed to UseExceptionHandler re-executes a page and is no handler"
    );
    let middleware = plan
        .requests
        .iter()
        .find(|p| {
            p.request["state"]["error_handler"]["registered"]
                .as_str()
                .is_some_and(|r| r.contains("middleware"))
        })
        .unwrap();
    let state = &middleware.request["state"];
    assert!(
        state["error_handler"]["helpers"][0]
            .as_str()
            .unwrap()
            .contains("WriteAsync(exception.Message)")
    );
    let classes = state["error_classes"].as_str().unwrap();
    assert!(
        classes.starts_with("public class DuplicateException : Exception\n{"),
        "{classes}"
    );
    assert!(
        classes.ends_with(
            "public class NotFoundException(string name) : Exception($\"{name} was not found\");"
        ),
        "{classes}"
    );
}
