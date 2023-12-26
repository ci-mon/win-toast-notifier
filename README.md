[![Build status](https://github.com/ci-mon/win-toast-notifier/actions/workflows/rust.yml/badge.svg)](https://github.com/ci-mon/win-toast-notifier/actions/workflows/rust.yml)

# Win toast notifier - easy windows toast notifications
Toast notification app with HTTP API

## Command line API
```
.\win-toast-notifier.exe help
Provides HTTP API to windows toast notifications interop

Usage: win-toast-notifier.exe <COMMAND>

Commands:
  register     Registers application_id in registry. Requires admin rights
  un-register  Removes application_id registration in registry
  test         Creates sample notification
  listen       Starts HTTP API
  help         Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

## HTTP API
```http request
POST http://localhost:7070/notify
Api-Key: 1
Content-Type: application/json

{
    "toast_xml_path": "C:\\Rust\\win-toast-notifier\\toast.xml"
}
```

```http request
POST http://localhost:7070/notify
Content-Type: application/json

{
    "toast_xml": "<toast><visual><binding template=\"ToastGeneric\"><text>Hello World</text><text>This is a simple toast message</text></binding></visual></toast>"
}
```

```http request
DELETE http://localhost:7070/notification?id=3
```

```http request
DELETE http://localhost:7070/all
```

```http request
GET http://localhost:7070/quit

Api-Key: 1
```

```http request
GET http://localhost:7070/status-stream?from=2

Api-Key: 1
```
