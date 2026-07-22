#define PY_SSIZE_T_CLEAN
#include <Python.h>
#include <interfaces/highs_c_api.h>

static PyObject *
demo_version(PyObject *self, PyObject *args)
{
    return Py_BuildValue(
        "(iii)", Highs_versionMajor(), Highs_versionMinor(), Highs_versionPatch());
}

static PyObject *
demo_create_and_destroy(PyObject *self, PyObject *args)
{
    void *highs = Highs_create();
    if (highs == NULL) {
        PyErr_SetString(PyExc_RuntimeError, "Highs_create returned NULL");
        return NULL;
    }
    Highs_destroy(highs);
    Py_RETURN_NONE;
}

static PyMethodDef DemoMethods[] = {
    {"version", demo_version, METH_NOARGS, "Return HiGHS's (major, minor, patch) version."},
    {"create_and_destroy", demo_create_and_destroy, METH_NOARGS,
     "Create and destroy a Highs instance, proving the solver actually links and runs."},
    {NULL, NULL, 0, NULL},
};

static struct PyModuleDef demomodule = {
    PyModuleDef_HEAD_INIT,
    "_demo",
    NULL,
    -1,
    DemoMethods,
};

PyMODINIT_FUNC
PyInit__demo(void)
{
    return PyModule_Create(&demomodule);
}
