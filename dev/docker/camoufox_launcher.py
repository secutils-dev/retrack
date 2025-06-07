import os
from camoufox.server import launch_server

# Get port from environment variable.
try:
    port = int(os.environ.get("CAMOUFOX_PORT", "7777"))
except ValueError:
    raise ValueError(f"Invalid CAMOUFOX_PORT value: {os.environ.get('PORT')}. Please provide a valid integer.")

def str_to_bool(val):
    return str(val).lower() in ("true", "1", "yes")

headless_env = os.environ.get("CAMOUFOX_HEADLESS", "virtual").lower()

launch_server(
    headless="virtual" if headless_env == "virtual" else str_to_bool(headless_env),
    main_world_eval=str_to_bool(os.environ.get("CAMOUFOX_USE_MAIN_WORLD", "True")),
    port=port,
    host="0.0.0.0",
    ws_path=os.environ.get("CAMOUFOX_WS_PATH", "camoufox")
)
