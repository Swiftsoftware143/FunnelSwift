#!/bin/bash
NGINX_PID=
if [ -n  ]; then
    kill -HUP 
    echo Nginx reloaded (PID )
else
    nginx
    echo Nginx started
fi
