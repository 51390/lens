AC_INIT([prism], [0.1])
AC_CONFIG_SRCDIR([src])
AM_INIT_AUTOMAKE([-Wall foreign])
LT_INIT
AC_PROG_CXX
AC_CONFIG_FILES([Makefile])
PKG_CHECK_MODULES(libecap, [libecap >= 1.0 libecap < 1.1])
AC_CHECK_HEADER(
                zlib.h,
                AC_CHECK_LIB(
                             z,
                             [deflate],
                             ,
                             AC_MSG_ERROR("libz not found or wrong version")
                ),
                AC_MSG_ERROR("zlib.h not found")
)
AC_OUTPUT
