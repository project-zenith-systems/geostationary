## Add graceful shutdown on CTRL-C

The dedicated server (`geostationary-server`) should handle SIGINT/CTRL-C gracefully — flush pending network messages, notify connected clients, and shut down cleanly instead of terminating abruptly.
