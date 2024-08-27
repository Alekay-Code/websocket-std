#ifndef _WEBSOCKET_STD_H
#define _WEBSOCKET_STD_H

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

// TODO: Change the name of the enum that doesn't contain the error word (to not be confused if this value is an error or not)
typedef enum { 
    WSStatusOK,
    WSStatusUnreachableHost,
    WSStatusHandShakeError,
    WSStatusInvalidFrame,
    WSStatusConnectionCloseError,
    WSStatusDecodingFromUTF8Error,
    WSStatusIOError, 
} WSStatus;

typedef enum {
    WSREASON_SERVER_CLOSED,
    WSREASON_CLIENT_CLOSED
} WSReason;

typedef struct {
    WSReason reason;
    uint16_t status;
} WSReason_t;

typedef const void* RustEvent;

typedef enum WSEventKind { 
    WSEvent_CONNECT,
    WSEvent_TEXT,
    WSEvent_CLOSE,
} WSEventKind_t;

typedef struct WSEvent {
    WSEventKind_t kind;
    void* value; 
} WSEvent_t;

typedef struct {} WSSClient_t;

typedef struct {
  size_t len;
  char** p;
} Protocols_t;

#define PROTOCOLS(...) { \
    .p = (char*[]){__VA_ARGS__}, \
    .len = sizeof((char*[]){__VA_ARGS__}) / sizeof(char*) \
}

typedef void (*ws_handler_t)(WSSClient_t*, RustEvent, void*);

typedef struct {
  ws_handler_t callback;
  Protocols_t protocols;  
} WSSConfig_t;

/*
* Creates a new WSSClient_t or NULL if an error occurred
*/
WSSClient_t *wssclient_new(void);


/*
* Init the websocket, connecting to the given host
* 
* Parameters:
* - WSSClient_t* client
* - const char* url: Websocket server URL: <ws/wss>://<host>:<port>/<path>
* - ws_handler_t* callback: Callback to execute when an events comes 
*
*/
void wssclient_init(WSSClient_t *client,
                    const char *url,
                    WSSConfig_t config);               

/*
* Function to execute the internal event loop of the websocket 
* 
* Parameters:
* - WSSClient_t* client
*
* Return:
* Internal state of the websocket, just to know if it is fine or something happened during some operation.
*
*/
WSStatus wssclient_loop(WSSClient_t* client);


/*
* Add a new event in the websocket to send the given message (Text)
* 
* Parameters:
* - WSSClient_t* client
* - message: string to send
*
*/
void wssclient_send(WSSClient_t* client, const char* message);


/*
* Drop the websocket from memory and close the connection with the server (graceful shutdown)
* 
* Parameters:
* - WSSClient_t* client
*
*/
void wssclient_drop(WSSClient_t* client);


/*
* Returns the accepted protocol or null if the server didn't accepted any protocol 
* 
* Parameters:
* - WSSClient_t* client
*
*/
char* wssclient_protocol(WSSClient_t* client);

WSEvent_t from_rust_event(RustEvent event);

#endif
