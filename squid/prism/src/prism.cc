#include <cstring>
#include <dlfcn.h>
#include <gperftools/heap-checker.h>
#include <iostream>
#include <libecap/common/autoconf.h>
#include <libecap/common/registry.h>
#include <libecap/common/errors.h>
#include <libecap/common/message.h>
#include <libecap/common/header.h>
#include <libecap/common/names.h>
#include <libecap/common/named_values.h>
#include <libecap/host/host.h>
#include <libecap/adapter/service.h>
#include <libecap/adapter/xaction.h>
#include <libecap/host/xaction.h>
#include <pthread.h>

namespace Adapter {

using libecap::size_type;

libecap::Name headerContentEncoding("Content-Encoding", libecap::Name::NextId());

typedef struct {
    size_t size;
    const void* bytes;
} Chunk;


class Service: public libecap::adapter::Service {
	public:
        Service();
		// About
		virtual std::string uri() const; // unique across all vendors
		virtual std::string tag() const; // changes with version and config
		virtual void describe(std::ostream &os) const; // free-format info

		// Configuration
		virtual void configure(const libecap::Options &cfg);
		virtual void reconfigure(const libecap::Options &cfg);
		void setOne(const libecap::Name &name, const libecap::Area &valArea);

		// Lifecycle
		virtual void start(); // expect makeXaction() calls
		virtual void stop(); // no more makeXaction() calls until start()
		virtual void retire(); // no more makeXaction() calls

		// Scope (XXX: this may be changed to look at the whole header)
		virtual bool wantsUrl(const char *url) const;

		// Work
		virtual MadeXactionPointer makeXaction(libecap::host::Xaction *hostx);

        void (*init)();
        void (*transfer)(int, const void*, size_t, const char*);
        void (*commit)(int, const char*, const char*);
        void (*header)(int, const char*, const char*, const char*);
        void (*content_done)(int);
        Chunk (*get_content)(int);
    private:
        void * module_;
        std::string analyzerPath;
};


class Cfgtor: public libecap::NamedValueVisitor {
	public:
		Cfgtor(Service &aSvc): svc(aSvc) {}
		virtual void visit(const libecap::Name &name, const libecap::Area &value) {
			svc.setOne(name, value);
		}
		Service &svc;
};

class HeaderVisitor: public libecap::NamedValueVisitor {
    public:
        HeaderVisitor(libecap::shared_ptr<const Service> aSvc, int anId, const char* anUri): svc(aSvc), id(anId), requestUri(anUri) {}
        virtual void visit(const libecap::Name &name, const libecap::Area &value) {
            svc->header(id, name.image().c_str(), value.start, requestUri);
        }

    private:
        libecap::shared_ptr<const Service> svc;
        int id;
        const char* requestUri;
};


class Xaction: public libecap::adapter::Xaction {
	public:
		Xaction(libecap::shared_ptr<Service> s, libecap::host::Xaction *x);
		virtual ~Xaction();

		// meta-information for the host transaction
		virtual const libecap::Area option(const libecap::Name &name) const;
		virtual void visitEachOption(libecap::NamedValueVisitor &visitor) const;

		// lifecycle
		virtual void start();
		virtual void stop();

		// adapted body transmission control
		virtual void abDiscard();
		virtual void abMake();
		virtual void abMakeMore();
		virtual void abStopMaking();

		// adapted body content extraction and consumption
		virtual libecap::Area abContent(size_type offset, size_type size);
		virtual void abContentShift(size_type size);

		// virgin body state notification
		virtual void noteVbContentDone(bool atEnd);
		virtual void noteVbContentAvailable();

	protected:
		void stopVb();
		libecap::host::Xaction *lastHostCall();

	private:
        void _processBuffers();
        void _cleanup();

		libecap::shared_ptr<const Service> service;
        libecap::shared_ptr<libecap::Message>  adapted;
		libecap::host::Xaction *hostx;
                                
        int id = 0;
        int contentLength = 0;
        char* requestUri = 0;

		typedef enum { opUndecided, opOn, opComplete, opNever } OperationState;
};

static const std::string CfgErrorPrefix =
	"Modifying Adapter: configuration error: ";

} // namespace Adapter

Adapter::Service::Service(): libecap::adapter::Service() {
}

std::string Adapter::Service::uri() const {
	return "ecap://e-cap.org/ecap/services/51390/prism";
}

std::string Adapter::Service::tag() const {
	return PACKAGE_VERSION;
}

void Adapter::Service::describe(std::ostream &os) const {
	os << "A modifying adapter from " << PACKAGE_NAME << " v" << PACKAGE_VERSION;
}

void Adapter::Service::configure(const libecap::Options &cfg) {
	Cfgtor cfgtor(*this);
	cfg.visitEachOption(cfgtor);
}

void Adapter::Service::reconfigure(const libecap::Options &cfg) {
	configure(cfg);
}

void Adapter::Service::setOne(const libecap::Name &key, const libecap::Area &val) {
	const std::string value = val.toString();
    const std::string name = key.image();
    if (key.assignedHostId()) {
		// skip host-standard options we do not know or care about
    } else if(name == "analyzerPath") {
        analyzerPath = value;
    } else
		throw libecap::TextException(CfgErrorPrefix +
			"unsupported configuration parameter: " + name);
}

void Adapter::Service::start() {
	libecap::adapter::Service::start();

    std::clog << "Prism starting" << std::endl;

    module_ = dlopen(analyzerPath.c_str(), RTLD_NOW | RTLD_GLOBAL);

    if(module_) {
        init = (void (*)())dlsym(module_, "init");
        init();

        transfer = (void (*)(int, const void*, size_t, const char*))dlsym(module_, "transfer");
        commit = (void (*)(int, const char*, const char*))dlsym(module_, "commit");
        header = (void (*)(int, const char*, const char*, const char*))dlsym(module_, "header");
        get_content = (Chunk (*)(int))dlsym(module_, "get_content");
        content_done = (void (*)(int))dlsym(module_, "content_done");
    }

    std::clog << "Prism init OK" << std::endl;
}

void Adapter::Service::stop() {
	// custom code would go here, but this service does not have one
	libecap::adapter::Service::stop();
}

void Adapter::Service::retire() {
	// custom code would go here, but this service does not have one
	libecap::adapter::Service::stop();
}

bool Adapter::Service::wantsUrl(const char *url) const {
	return true; // no-op is applied to all messages
}

Adapter::Service::MadeXactionPointer
Adapter::Service::makeXaction(libecap::host::Xaction *hostx) {
	return Adapter::Service::MadeXactionPointer(
		new Adapter::Xaction(std::tr1::static_pointer_cast<Service>(self), hostx));
}

int counter = 0;

Adapter::Xaction::Xaction(libecap::shared_ptr<Service> aService,
	libecap::host::Xaction *x):
	service(aService),
	hostx(x) {
    id = counter++;
}

Adapter::Xaction::~Xaction() {
	if (libecap::host::Xaction *x = hostx) {
		hostx = 0;
		x->adaptationAborted();
	}

    service->commit(id, "N/A", requestUri);
    _cleanup();
}

const libecap::Area Adapter::Xaction::option(const libecap::Name &) const {
	return libecap::Area(); // this transaction has no meta-information
}

void Adapter::Xaction::visitEachOption(libecap::NamedValueVisitor &) const {
	// this transaction has no meta-information to pass to the visitor
}


void Adapter::Xaction::start() {
    if(!hostx) {
        return;
    }
	if (hostx->virgin().body()) {
		hostx->vbMake(); 
	}

	adapted = hostx->virgin().clone();

    if(!adapted) {
        return;
    }

    if(adapted->header().hasAny(libecap::headerContentLength)) {
        adapted->header().removeAny(libecap::headerContentLength);
    }

	if (!adapted->body()) {
		lastHostCall()->useAdapted(adapted);
	} else {
		hostx->useAdapted(adapted);
	}

}

void Adapter::Xaction::stop() {
	hostx = 0;
	// the caller will delete
    service->commit(id, "N/A", requestUri);
    _cleanup();
}

void Adapter::Xaction::_cleanup() {
    if(requestUri) {
        free(requestUri);
        requestUri = 0;
    }
}

void Adapter::Xaction::abDiscard()
{
	stopVb();
}

void Adapter::Xaction::abMake()
{
    hostx->noteAbContentAvailable();
}

void Adapter::Xaction::abMakeMore()
{
	hostx->vbMakeMore();
}

void Adapter::Xaction::abStopMaking()
{
	stopVb();
}

void to_file(const char* fname, int id, const char* suffix, const void* content, size_t n, bool append) {
    /*
    FILE *f;
    char* filename = (char*)malloc(1024);
    snprintf(filename, 1024, "%s-%d%s", fname, id, suffix);

    if(append) {
        f = fopen(filename, "a+");
    } else {
        f = fopen(filename, "w+");
    }

    fwrite(content, sizeof(char), n, f);

    free(filename);
    fclose(f);
    */
}

libecap::Area Adapter::Xaction::abContent(size_type offset, size_type size) {
    Chunk c = service->get_content(id);

    if(c.size) {
        return libecap::Area::FromTempBuffer((const char*)c.bytes, c.size);;
    } else {
        /*char z[4096];
        memset(z, 0, sizeof(z));

        libecap::Area a = libecap::Area::FromTempBuffer((const char*)z, 4096);
        return a;*/
        return libecap::Area();
    }
}

void Adapter::Xaction::abContentShift(size_type size) {
}

void Adapter::Xaction::noteVbContentDone(bool atEnd)
{
	stopVb();
    service->content_done(id);
    hostx->noteAbContentDone(atEnd);
}

void Adapter::Xaction::noteVbContentAvailable()
{

    if(!requestUri) {
        // grab the request uri
        const libecap::Message& cause = hostx->cause();
        const libecap::RequestLine& rl = dynamic_cast<const libecap::RequestLine&>(cause.firstLine());
        const libecap::Area uri = rl.uri();
        requestUri = (char*)malloc(uri.size + 1);
        memset(requestUri, 0, uri.size + 1);
        memcpy(requestUri, uri.start, uri.size);

        HeaderVisitor hv(service, id, requestUri);
        adapted->header().visitEach(hv);

    }

	const libecap::Area vb = hostx->vbContent(0, libecap::nsize); // get all vb
    
	hostx->vbContentShift(vb.size); // we have a copy; do not need vb any more

    if (vb.size && vb.start) {
        service->transfer(id, vb.start, vb.size, requestUri);
        hostx->noteAbContentAvailable();
    }
}

// tells the host that we are not interested in [more] vb
// if the host does not know that already
void Adapter::Xaction::stopVb() {
    hostx->vbStopMaking(); // we will not call vbContent() any more
}

// this method is used to make the last call to hostx transaction
// last call may delete adapter transaction if the host no longer needs it
// TODO: replace with hostx-independent "done" method
libecap::host::Xaction *Adapter::Xaction::lastHostCall() {
	libecap::host::Xaction *x = hostx;
	hostx = 0;
	return x;
}

// create the adapter and register with libecap to reach the host application
static const bool Registered =
	libecap::RegisterVersionedService(new Adapter::Service);
