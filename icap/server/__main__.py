#!/bin/env python
# -*- coding: utf8 -*-

import random
import SocketServer

from pyicap import *

class ThreadingSimpleServer(SocketServer.ThreadingMixIn, ICAPServer):
    pass

class ICAPHandler(BaseICAPRequestHandler):

    def request_OPTIONS(self):
        self.set_icap_response(200)
        self.set_icap_header('Methods', 'RESPMOD')
        self.set_icap_header('Preview', '0')
        self.send_headers(False)
        print('Got OPTIONS call')

    def request_RESPMOD(self):
        self.no_adaptation_required()
        print('Got RESPMOD call')

    def request_REQMOD(self):
        self.no_adaptation_required()
        print('Got REQMOD call')

port = 13440

server = ThreadingSimpleServer(('', port), ICAPHandler)
try:
    while 1:
        server.handle_request()
except KeyboardInterrupt:
    print("Finished")
