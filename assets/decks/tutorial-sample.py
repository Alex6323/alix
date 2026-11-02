def greet(name, excited=False):
    """Greet name; an empty name greets nobody in particular."""
    if not name:
        return "Hello!"
    ending = "!" if excited else "."
    return f"Hello, {name}{ending}"
